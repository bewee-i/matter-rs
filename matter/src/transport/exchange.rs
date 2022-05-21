use boxslab::Slab;
use colored::*;
use log::{error, info, trace};
use std::fmt;
use std::{any::Any, ops::DerefMut};

use crate::error::Error;
use crate::secure_channel;

use heapless::LinearMap;

use super::packet::PacketPool;
use super::session::CloneData;
use super::{
    mrp::ReliableMessage,
    packet::Packet,
    session::SessionHandle,
    session::{Session, SessionMgr},
};

pub struct ExchangeCtx<'a> {
    pub exch: &'a mut Exchange,
    pub sess: SessionHandle<'a>,
}

impl<'a> ExchangeCtx<'a> {
    pub fn send(&mut self, proto_tx: &mut Packet) -> Result<(), Error> {
        self.exch.send(proto_tx, self.sess.deref_mut())
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum ExchangeRole {
    Initiator = 0,
    Responder = 1,
}

impl Default for ExchangeRole {
    fn default() -> Self {
        ExchangeRole::Initiator
    }
}

#[derive(Debug, Default)]
pub struct Exchange {
    id: u16,
    sess_id: u16,
    role: ExchangeRole,
    // The number of users currently using this exchange. This will go away when
    // we start using Arc/Rc and the Exchange object itself is dynamically allocated
    // But, maybe that never happens
    user_cnt: u8,
    // Currently I see this primarily used in PASE and CASE. If that is the limited use
    // of this, we might move this into a separate data structure, so as not to burden
    // all 'exchanges'.
    data: Option<Box<dyn Any>>,
    mrp: ReliableMessage,
}

impl Exchange {
    pub fn new(id: u16, sess_id: u16, role: ExchangeRole) -> Exchange {
        Exchange {
            id,
            sess_id,
            role,
            user_cnt: 1,
            data: None,
            mrp: ReliableMessage::new(),
        }
    }

    pub fn close(&mut self) {
        self.data = None;
        self.release();
    }

    pub fn acquire(&mut self) {
        self.user_cnt += 1;
    }

    pub fn release(&mut self) {
        self.user_cnt -= 1;
        // Even if we get to a zero reference count, because the memory is static,
        // an exchange manager purge call is required to clean us up
    }

    pub fn is_purgeable(&self) -> bool {
        // No Users, No pending ACKs/Retrans
        self.user_cnt == 0 && self.mrp.is_empty()
    }

    pub fn get_id(&self) -> u16 {
        self.id
    }

    pub fn get_role(&self) -> ExchangeRole {
        self.role
    }

    pub fn set_exchange_data(&mut self, data: Box<dyn Any>) {
        self.data = Some(data);
    }

    pub fn clear_exchange_data(&mut self) {
        self.data = None;
    }

    pub fn get_exchange_data<T: Any>(&mut self) -> Option<&mut T> {
        self.data.as_mut()?.downcast_mut::<T>()
    }

    pub fn take_exchange_data<T: Any>(&mut self) -> Option<Box<T>> {
        self.data.take()?.downcast::<T>().ok()
    }

    pub fn send(&mut self, proto_tx: &mut Packet, session: &mut Session) -> Result<(), Error> {
        trace!("payload: {:x?}", proto_tx.as_borrow_slice());
        info!(
            "{} with proto id: {} opcode: {}",
            "Sending".blue(),
            proto_tx.get_proto_id(),
            proto_tx.get_proto_opcode(),
        );

        if self.sess_id != session.get_local_sess_id() {
            error!("This should have never happened");
            return Err(Error::InvalidState);
        }
        proto_tx.proto.exch_id = self.id;
        if self.role == ExchangeRole::Initiator {
            proto_tx.proto.set_initiator();
        }

        session.pre_send(proto_tx)?;
        self.mrp.pre_send(proto_tx)?;
        session.send(proto_tx)
    }
}

impl fmt::Display for Exchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "exch_id: {:?}, sess_id: {}, role: {:?}, data: {:?}, use_cnt: {} mrp: {:?}",
            self.id, self.sess_id, self.role, self.data, self.user_cnt, self.mrp,
        )
    }
}

pub fn get_role(is_initiator: bool) -> ExchangeRole {
    if is_initiator {
        ExchangeRole::Initiator
    } else {
        ExchangeRole::Responder
    }
}

pub fn get_complementary_role(is_initiator: bool) -> ExchangeRole {
    if is_initiator {
        ExchangeRole::Responder
    } else {
        ExchangeRole::Initiator
    }
}

const MAX_EXCHANGES: usize = 8;

#[derive(Default)]
pub struct ExchangeMgr {
    // keys: exch-id
    exchanges: LinearMap<u16, Exchange, MAX_EXCHANGES>,
    sess_mgr: SessionMgr,
}

pub const MAX_MRP_ENTRIES: usize = 4;

impl ExchangeMgr {
    pub fn new(sess_mgr: SessionMgr) -> Self {
        Self {
            sess_mgr,
            exchanges: Default::default(),
        }
    }

    pub fn get_sess_mgr(&mut self) -> &mut SessionMgr {
        &mut self.sess_mgr
    }

    pub fn _get_with_id(
        exchanges: &mut LinearMap<u16, Exchange, MAX_EXCHANGES>,
        exch_id: u16,
    ) -> Option<&mut Exchange> {
        exchanges.get_mut(&exch_id)
    }

    pub fn get_with_id(&mut self, exch_id: u16) -> Option<&mut Exchange> {
        ExchangeMgr::_get_with_id(&mut self.exchanges, exch_id)
    }

    pub fn _get(
        exchanges: &mut LinearMap<u16, Exchange, MAX_EXCHANGES>,
        sess_id: u16,
        id: u16,
        role: ExchangeRole,
        create_new: bool,
    ) -> Result<&mut Exchange, Error> {
        // I don't prefer that we scan the list twice here (once for contains_key and other)
        if !exchanges.contains_key(&(id)) {
            if create_new {
                // If an exchange doesn't exist, create a new one
                info!("Creating new exchange");
                let e = Exchange::new(id, sess_id, role);
                if exchanges.insert(id, e).is_err() {
                    return Err(Error::NoSpace);
                }
            } else {
                return Err(Error::NoSpace);
            }
        }

        // At this point, we would either have inserted the record if 'create_new' was set
        // or it existed already
        if let Some(result) = exchanges.get_mut(&id) {
            if result.get_role() == role && sess_id == result.sess_id {
                Ok(result)
            } else {
                Err(Error::NoExchange)
            }
        } else {
            error!("This should never happen");
            Err(Error::NoSpace)
        }
    }
    pub fn get(
        &mut self,
        local_sess_id: u16,
        id: u16,
        role: ExchangeRole,
        create_new: bool,
    ) -> Result<&mut Exchange, Error> {
        ExchangeMgr::_get(&mut self.exchanges, local_sess_id, id, role, create_new)
    }

    pub fn recv(&mut self, proto_rx: &mut Packet) -> Result<ExchangeCtx, Error> {
        // Get the session
        let s = match self.sess_mgr.recv(proto_rx) {
            Ok(s) => s,
            Err(Error::NoSpace) => {
                let evict_index = self.sess_mgr.get_lru();
                self.evict_session(evict_index)?;
                info!("Reattempting session creation");
                self.sess_mgr.recv(proto_rx)?
            }
            Err(e) => {
                return Err(e);
            }
        };

        let mut session = self.sess_mgr.get_session_handle(s);

        // Decrypt the message
        session.recv(proto_rx)?;

        // Get the exchange
        let exch = ExchangeMgr::_get(
            &mut self.exchanges,
            proto_rx.plain.sess_id,
            proto_rx.proto.exch_id,
            get_complementary_role(proto_rx.proto.is_initiator()),
            // We create a new exchange, only if the peer is the initiator
            proto_rx.proto.is_initiator(),
        )?;

        // Message Reliability Protocol
        exch.mrp.recv(&proto_rx)?;

        Ok(ExchangeCtx {
            exch,
            sess: session,
        })
    }

    pub fn send(&mut self, exch_id: u16, proto_tx: &mut Packet) -> Result<(), Error> {
        let exchange =
            ExchangeMgr::_get_with_id(&mut self.exchanges, exch_id).ok_or(Error::NoExchange)?;
        let mut session = self
            .sess_mgr
            .get_with_id(exchange.sess_id)
            .ok_or(Error::NoSession)?;
        exchange.send(proto_tx, &mut session)
    }

    pub fn purge(&mut self) {
        let mut to_purge: LinearMap<u16, (), MAX_EXCHANGES> = LinearMap::new();

        for (exch_id, exchange) in self.exchanges.iter() {
            if exchange.is_purgeable() {
                let _ = to_purge.insert(*exch_id, ());
            }
        }
        for (exch_id, _) in to_purge.iter() {
            self.exchanges.remove(&*exch_id);
        }
    }

    pub fn pending_acks(&mut self, expired_entries: &mut LinearMap<u16, (), MAX_MRP_ENTRIES>) {
        for (exch_id, exchange) in self.exchanges.iter() {
            if exchange.mrp.is_ack_ready() {
                expired_entries.insert(*exch_id, ()).unwrap();
            }
        }
    }

    pub fn evict_session(&mut self, index: usize) -> Result<(), Error> {
        info!("Sessions full, vacating session with index: {}", index);
        // If we enter here, we have an LRU session that needs to be reclaimed
        // As per the spec, we need to send a CLOSE here

        let session = self.sess_mgr.mut_by_index(index).ok_or(Error::Invalid)?;
        let mut tx = Slab::<PacketPool>::new(Packet::new_tx()?).ok_or(Error::NoSpace)?;
        secure_channel::common::create_sc_status_report(
            &mut tx,
            secure_channel::common::SCStatusCodes::CloseSession,
            None,
        )?;

        let sess_id = session.get_local_sess_id();

        if let Some((_, exchange)) =
            self.exchanges
                .iter_mut()
                .find(|(_, e)| if e.sess_id == sess_id { true } else { false })
        {
            // Send Close_session on this exchange, and then close the session
            // Should this be done for all exchanges?
            error!("Sending Close Session");
            exchange.send(&mut tx, session)?;
            // TODO: This wouldn't actually send it out, because 'transport' isn't owned yet.
        }

        let remove_exchanges: Vec<u16> = self
            .exchanges
            .iter()
            .filter_map(|(eid, e)| {
                if e.sess_id == sess_id {
                    Some(*eid)
                } else {
                    None
                }
            })
            .collect();
        info!(
            "Terminating the following exchanges: {:?}",
            remove_exchanges
        );
        for exch_id in remove_exchanges {
            // Remove from exchange list
            self.exchanges.remove(&exch_id);
        }
        self.sess_mgr.remove(index);
        Ok(())
    }

    pub fn add_session(&mut self, clone_data: CloneData) -> Result<SessionHandle, Error> {
        let sess_idx = match self.sess_mgr.clone_session(&clone_data) {
            Ok(idx) => idx,
            Err(Error::NoSpace) => {
                let evict_index = self.sess_mgr.get_lru();
                self.evict_session(evict_index)?;
                self.sess_mgr.clone_session(&clone_data)?
            }
            Err(e) => {
                return Err(e);
            }
        };
        Ok(self.sess_mgr.get_session_handle(sess_idx))
    }
}

impl fmt::Display for ExchangeMgr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{  Session Mgr: {},", self.sess_mgr)?;
        writeln!(f, "  Exchanges: [")?;
        for s in &self.exchanges {
            writeln!(f, "{{ {}, }},", s.1)?;
        }
        writeln!(f, "  ]")?;
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use crate::{
        error::Error,
        transport::session::{CloneData, SessionMgr, SessionMode, MAX_SESSIONS},
    };

    use super::{ExchangeMgr, ExchangeRole};

    #[test]
    fn test_purge() {
        let sess_mgr = SessionMgr::new();
        let mut mgr = ExchangeMgr::new(sess_mgr);
        let _ = mgr.get(1, 2, ExchangeRole::Responder, true).unwrap();
        let _ = mgr.get(1, 3, ExchangeRole::Responder, true).unwrap();

        mgr.purge();
        assert_eq!(mgr.get_with_id(2).is_some(), true);
        assert_eq!(mgr.get_with_id(3).is_some(), true);

        // Release e1
        let e1 = mgr.get_with_id(2).unwrap();
        e1.release();
        mgr.purge();
        assert_eq!(mgr.get_with_id(2).is_some(), false);
        assert_eq!(mgr.get_with_id(3).is_some(), true);

        // Acquire e2
        let e2 = mgr.get_with_id(3).unwrap();
        e2.acquire();
        mgr.purge();
        assert_eq!(mgr.get_with_id(3).is_some(), true);

        // Release e2 once
        let e2 = mgr.get_with_id(3).unwrap();
        e2.release();
        mgr.purge();
        assert_eq!(mgr.get_with_id(3).is_some(), true);

        // Release e2 again
        let e2 = mgr.get_with_id(3).unwrap();
        e2.release();
        mgr.purge();
        assert_eq!(mgr.get_with_id(3).is_some(), false);
    }

    fn get_clone_data(peer_sess_id: u16, local_sess_id: u16) -> CloneData {
        CloneData::new(
            12341234,
            43211234,
            peer_sess_id,
            local_sess_id,
            SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 10, 1)), 8080),
            SessionMode::Pase,
        )
    }

    fn fill_sessions(mgr: &mut ExchangeMgr, count: usize) {
        let mut local_sess_id = 1;
        let mut peer_sess_id = 100;
        for _ in 1..count {
            let clone_data = get_clone_data(peer_sess_id, local_sess_id);
            match mgr.add_session(clone_data) {
                Ok(s) => (assert_eq!(peer_sess_id, s.get_peer_sess_id())),
                Err(Error::NoSpace) => break,
                _ => {
                    panic!("Couldn't, create session");
                }
            }
            local_sess_id += 1;
            peer_sess_id += 1;
        }
    }

    #[test]
    /// We purposefuly overflow the sessions
    /// and when the overflow happens, we confirm that
    /// - The sessions are evicted in LRU
    /// - The exchanges associated with those sessions are evicted too
    fn test_sess_evict() {
        let sess_mgr = SessionMgr::new();
        let mut mgr = ExchangeMgr::new(sess_mgr);

        fill_sessions(&mut mgr, MAX_SESSIONS + 1);
        // Sessions are now full from local session id 1 to 16

        // Create exchanges for sessions 2 and 3
        //   Exchange IDs are 20 and 30 respectively
        let _ = mgr.get(2, 20, ExchangeRole::Responder, true).unwrap();
        let _ = mgr.get(3, 30, ExchangeRole::Responder, true).unwrap();

        // Confirm that session ids 1 to MAX_SESSIONS exists
        for i in 1..(MAX_SESSIONS + 1) {
            assert_eq!(mgr.sess_mgr.get_with_id(i as u16).is_none(), false);
        }
        // Confirm that the exchanges are around
        assert_eq!(mgr.get_with_id(20).is_none(), false);
        assert_eq!(mgr.get_with_id(30).is_none(), false);

        let mut old_local_sess_id = 1;
        let mut new_local_sess_id = 100;
        let mut new_peer_sess_id = 200;

        for i in 1..(MAX_SESSIONS + 1) {
            // Now purposefully overflow the sessions by adding another session
            let session = mgr
                .add_session(get_clone_data(new_peer_sess_id, new_local_sess_id))
                .unwrap();
            assert_eq!(session.get_peer_sess_id(), new_peer_sess_id);

            // This should have evicted session with local sess_id
            assert_eq!(mgr.sess_mgr.get_with_id(old_local_sess_id).is_none(), true);

            new_local_sess_id += 1;
            new_peer_sess_id += 1;
            old_local_sess_id += 1;

            match i {
                1 => {
                    // Both exchanges should exist
                    assert_eq!(mgr.get_with_id(20).is_none(), false);
                    assert_eq!(mgr.get_with_id(30).is_none(), false);
                }
                2 => {
                    // Exchange 20 would have been evicted
                    assert_eq!(mgr.get_with_id(20).is_none(), true);
                    assert_eq!(mgr.get_with_id(30).is_none(), false);
                }
                3 => {
                    // Exchange 20 and 30 would have been evicted
                    assert_eq!(mgr.get_with_id(20).is_none(), true);
                    assert_eq!(mgr.get_with_id(30).is_none(), true);
                }
                _ => {}
            }
        }
        //        println!("Session mgr {}", mgr.sess_mgr);
    }
}
