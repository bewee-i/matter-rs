use core::fmt;
use std::any::Any;

use crate::{error::*, transport::exchange::*};
use heapless::Vec;
use log::info;

const MATTER_AES128_KEY_SIZE: usize = 16;

const EXCHANGES_PER_SESSION: usize = 4;

#[derive(Debug, PartialEq)]
pub enum SessionMode {
    Encrypted,
    PlainText,
}

impl Default for SessionMode {
    fn default() -> Self {
        SessionMode::PlainText
    }
}

#[derive(Debug, Default)]
pub struct Session {
    // If this field is None, the rest of the members are ignored
    peer_addr: Option<std::net::IpAddr>,
    // I find the session initiator/responder role getting confused with exchange initiator/responder
    // So, we might keep this as enc_key and dec_key for now
    dec_key: [u8; MATTER_AES128_KEY_SIZE],
    enc_key: [u8; MATTER_AES128_KEY_SIZE],
    att_challenge: [u8; MATTER_AES128_KEY_SIZE],
    /*
     *
     * - Session Role (whether we are session-Initiator or Session-Responder (use the correct key accordingly(
     * - local session ID (our ID assigned to this session)
     * - peer session ID (the peer's ID assigned to this session)
     * - local message counter (the one we'll use for our TX)
     * - message reception state (a list of counters already received from the peer) to detect duplicates
     * - peer Node ID - instead of the IP Address, which can change, the Node ID should be used
     * - This is all for 'unicast' sessions
     */
    local_sess_id: u16,
    peer_sess_id: u16,
    // The local sess id is only set on session 0 of a peer-addr, when PASE/CASE is in-progress.
    // We could have held the local session ID in the PASE/CASE specific data, untill an encrypted
    // session is established. But doing that implies that the new session ID allocator couldn't
    // see this child session ID. Keeping it here, makes it easier to manage.
    child_local_sess_id: u16,
    msg_ctr: u32,
    exchanges: [Option<Exchange>; EXCHANGES_PER_SESSION],
    mode: SessionMode,
}

#[derive(Debug)]
pub struct CloneData {
    pub dec_key: [u8; MATTER_AES128_KEY_SIZE],
    pub enc_key: [u8; MATTER_AES128_KEY_SIZE],
    pub att_challenge: [u8; MATTER_AES128_KEY_SIZE],
    peer_sess_id: u16,
}
impl CloneData {
    pub fn new(peer_sess_id: u16) -> CloneData {
        CloneData {
            dec_key: [0; MATTER_AES128_KEY_SIZE],
            enc_key: [0; MATTER_AES128_KEY_SIZE],
            att_challenge: [0; MATTER_AES128_KEY_SIZE],
            peer_sess_id,
        }
    }
}

impl Session {
    // All new sessions begin life as PlainText, with a child local session ID,
    // then they eventually get converted into an encrypted session with the new_encrypted_session() which
    // clones from this plaintext session, but acquires the local/peer session IDs and the
    // encryption keys.
    pub fn new(child_local_sess_id: u16, peer_addr: std::net::IpAddr) -> Session {
        Session {
            peer_addr: Some(peer_addr),
            dec_key: [0; MATTER_AES128_KEY_SIZE],
            enc_key: [0; MATTER_AES128_KEY_SIZE],
            att_challenge: [0; MATTER_AES128_KEY_SIZE],
            child_local_sess_id,
            peer_sess_id: 0,
            local_sess_id: 0,
            msg_ctr: 1,
            exchanges: Default::default(),
            mode: SessionMode::PlainText,
        }
    }

    // A new encrypted session always clones from a previous 'new' session
    pub fn clone(&mut self, clone_from: &CloneData) -> Session {
        let session = Session {
            peer_addr: self.peer_addr,
            dec_key: clone_from.dec_key,
            enc_key: clone_from.enc_key,
            att_challenge: clone_from.att_challenge,
            local_sess_id: self.child_local_sess_id,
            peer_sess_id: clone_from.peer_sess_id,
            child_local_sess_id: 0,
            msg_ctr: 1,
            exchanges: Default::default(),
            mode: SessionMode::Encrypted,
        };

        self.child_local_sess_id = 0;

        session
    }

    pub fn get_exchange(&mut self, index: usize) -> Option<&mut Exchange> {
        if index < EXCHANGES_PER_SESSION {
            self.exchanges[index].as_mut()
        } else {
            None
        }
    }

    pub fn get_exchange_id(
        &mut self,
        id: u16,
        role: ExchangeRole,
        create_new: bool,
    ) -> Option<usize> {
        if let Some(index) = self.exchanges.iter().position(|x| {
            if let Some(x) = x {
                x.is_match(id, role)
            } else {
                false
            }
        }) {
            Some(index)
        } else if create_new {
            // If an exchange doesn't exist, create a new one
            info!("Creating new exchange");
            let e = Exchange::new(id, role);
            if let Some(index) = self.exchanges.iter().position(|x| x.is_none()) {
                // Return the exchange that was just added
                self.exchanges[index] = Some(e);
                Some(index)
            } else {
                None
            }
        } else {
            // Got a message that has no matching Exchange object
            None
        }
    }

    pub fn set_exchange_data(
        &mut self,
        exch_index: usize,
        data: Box<dyn Any>,
    ) -> Result<(), Error> {
        self.exchanges[exch_index]
            .as_mut()
            .and_then(|e| {
                e.set_exchange_data(data);
                Some(())
            })
            .ok_or(Error::NoExchange)
    }

    pub fn get_and_clear_exchange_data(&mut self, exch_index: usize) -> Option<Box<dyn Any>> {
        self.exchanges[exch_index]
            .as_mut()
            .and_then(|e| e.get_and_clear_exchange_data())
    }

    pub fn clear_exchange_data(&mut self, exch_index: usize) -> Result<(), Error> {
        self.exchanges[exch_index]
            .as_mut()
            .and_then(|e| {
                e.clear_exchange_data();
                Some(())
            })
            .ok_or(Error::NoExchange)
    }

    pub fn get_local_sess_id(&self) -> u16 {
        self.local_sess_id
    }

    pub fn get_child_local_sess_id(&self) -> u16 {
        self.child_local_sess_id
    }

    pub fn get_peer_sess_id(&self) -> u16 {
        self.peer_sess_id
    }

    pub fn set_local_sess_id(&mut self) {
        self.local_sess_id = self.child_local_sess_id;
    }

    // This is required for the bypass case
    pub fn cheat_set_zero_local_sess_id(&mut self) {
        self.local_sess_id = 0;
    }

    pub fn is_encrypted(&self) -> bool {
        self.mode == SessionMode::Encrypted
    }

    pub fn get_msg_ctr(&mut self) -> u32 {
        let ctr = self.msg_ctr;
        self.msg_ctr += 1;
        ctr
    }

    pub fn get_dec_key(&self) -> Option<&[u8]> {
        match self.mode {
            SessionMode::Encrypted => Some(&self.dec_key),
            SessionMode::PlainText => None,
        }
    }

    pub fn get_enc_key(&self) -> Option<&[u8]> {
        match self.mode {
            SessionMode::Encrypted => Some(&self.enc_key),
            SessionMode::PlainText => None,
        }
    }

    pub fn activate(
        &mut self,
        dec_key: &[u8],
        enc_key: &[u8],
        peer_sess_id: u16,
    ) -> Result<(), Error> {
        self.set_local_sess_id();
        self.peer_sess_id = peer_sess_id;
        if enc_key.len() == self.enc_key.len() {
            self.enc_key.copy_from_slice(enc_key);
        }
        if dec_key.len() == self.dec_key.len() {
            self.dec_key.copy_from_slice(dec_key);
        }
        self.mode = SessionMode::Encrypted;
        Ok(())
    }
}

impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "peer: {:?}, local: {}, remote: {}, msg_ctr: {}, mode: {:?}, exchanges: {:?}",
            self.peer_addr,
            self.local_sess_id,
            self.peer_sess_id,
            self.msg_ctr,
            self.mode,
            self.exchanges
        )
    }
}

#[derive(Debug)]
pub struct SessionMgr {
    next_sess_id: u16,
    sessions: Vec<Session, 16>,
}

impl SessionMgr {
    pub fn new() -> SessionMgr {
        SessionMgr {
            sessions: Vec::new(),
            next_sess_id: 1,
        }
    }

    fn get_next_sess_id(&mut self) -> u16 {
        let mut next_sess_id: u16;
        loop {
            next_sess_id = self.next_sess_id;

            // Increment next sess id
            self.next_sess_id = self.next_sess_id.overflowing_add(1).0;
            if self.next_sess_id == 0 {
                self.next_sess_id = 1;
            }

            // Ensure the currently selected id doesn't match any existing session
            if self.sessions.iter().position(|x| {
                x.local_sess_id == next_sess_id || x.child_local_sess_id == next_sess_id
            }) == None
            {
                break;
            }
        }
        next_sess_id
    }

    pub fn add(&mut self, peer_addr: std::net::IpAddr) -> Result<(usize, &mut Session), Error> {
        let child_sess_id = self.get_next_sess_id();
        let session = Session::new(child_sess_id, peer_addr);

        self.sessions.push(session).map_err(|_s| Error::NoSpace)?;
        let index = self._get(0, peer_addr, false).ok_or(Error::NoSpace)?;
        Ok((index, &mut self.sessions[index]))
    }

    pub fn add_session(&mut self, session: Session) -> Result<(), Error> {
        self.sessions.push(session).map_err(|_s| Error::NoSpace)
    }

    fn _get(&self, sess_id: u16, peer_addr: std::net::IpAddr, is_encrypted: bool) -> Option<usize> {
        let mode = if is_encrypted {
            SessionMode::Encrypted
        } else {
            SessionMode::PlainText
        };
        self.sessions.iter().position(|x| {
            x.local_sess_id == sess_id && x.peer_addr == Some(peer_addr) && x.mode == mode
        })
    }

    pub fn get(
        &mut self,
        sess_id: u16,
        peer_addr: std::net::IpAddr,
        is_encrypted: bool,
    ) -> Option<(usize, &mut Session)> {
        if let Some(index) = self._get(sess_id, peer_addr, is_encrypted) {
            Some((index, &mut self.sessions[index]))
        } else if sess_id == 0 && !is_encrypted {
            // We must create a new session for this case
            info!("Creating new session");
            self.add(peer_addr).ok()
        } else {
            None
        }
    }

    pub fn get_session(&mut self, sess_index: usize) -> Option<&mut Session> {
        Some(&mut self.sessions[sess_index])
    }

    pub fn get_exchange(&mut self, sess_index: usize, exch_index: usize) -> Option<&mut Exchange> {
        self.sessions[sess_index].exchanges[exch_index].as_mut()
    }
}

impl fmt::Display for SessionMgr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{[")?;
        for s in &self.sessions {
            writeln!(f, "{{ {}, }},", s)?;
        }
        write!(f, "], next_sess_id: {}", self.next_sess_id)?;
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::SessionMgr;
    use std::net::Ipv4Addr;

    #[test]
    fn test_next_sess_id_doesnt_reuse() {
        let mut sm = SessionMgr::new();
        sm.add(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
            .unwrap();
        assert_eq!(sm.get_next_sess_id(), 2);
        assert_eq!(sm.get_next_sess_id(), 3);
        sm.add(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
            .unwrap();
        assert_eq!(sm.get_next_sess_id(), 5);
    }

    #[test]
    fn test_next_sess_id_overflows() {
        let mut sm = SessionMgr::new();
        sm.add(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
            .unwrap();
        assert_eq!(sm.get_next_sess_id(), 2);
        sm.next_sess_id = 65534;
        assert_eq!(sm.get_next_sess_id(), 65534);
        assert_eq!(sm.get_next_sess_id(), 65535);
        assert_eq!(sm.get_next_sess_id(), 2);
    }
}
