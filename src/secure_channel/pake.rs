use super::spake2p::Spake2P;
use crate::error::Error;
use crate::proto_demux::ProtoCtx;
use crate::tlv::*;
use crate::tlv_common::TagType;
use crate::tlv_writer::TLVWriter;
use crate::transport::exchange::ExchangeRole;
use crate::transport::tx_ctx::TxCtx;
use log::{error, info};
use rand::prelude::*;

// This file basically deals with the handlers for the PASE secure channel protocol
// TLV extraction and encoding is done in this file.
// We create a Spake2p object and set it up in the exchange-data. This object then
// handles Spake2+ specific stuff.

// As per the spec the iteration count should be between 1000 and 100000
const ITERATION_COUNT: u32 = 2000;

// TODO: Password should be passed inside
const SPAKE2_PASSWORD: u32 = 123456;

#[derive(Default)]
pub struct PAKE {
    // As per the spec the salt should be between 16 to 32 bytes
    salt: [u8; 16],
    passwd: u32,
}

impl PAKE {
    pub fn new() -> Self {
        // TODO: Can any PBKDF2 calculation be pre-computed here
        let mut pake = PAKE {
            passwd: SPAKE2_PASSWORD,
            ..Default::default()
        };
        rand::thread_rng().fill_bytes(&mut pake.salt);
        pake
    }

    #[allow(non_snake_case)]
    pub fn handle_pasepake1(
        &mut self,
        proto_ctx: &mut ProtoCtx,
        tx_ctx: &mut TxCtx,
    ) -> Result<(), Error> {
        let mut spake2_boxed = proto_ctx
            .session
            .get_and_clear_exchange_data(proto_ctx.exch_id, ExchangeRole::Responder)
            .ok_or(Error::InvalidState)?;
        let spake2 = spake2_boxed
            .downcast_mut::<Spake2P>()
            .ok_or(Error::InvalidState)?;

        let pA = extract_pasepake1_params(proto_ctx.buf)?;
        let mut pB: [u8; 65] = [0; 65];
        let mut cB: [u8; 32] = [0; 32];
        spake2.start_verifier(self.passwd, ITERATION_COUNT, &self.salt)?;
        spake2.handle_pA(pA, &mut pB, &mut cB)?;

        let mut tlvwriter = TLVWriter::new(tx_ctx.get_write_buf());
        tlvwriter.put_start_struct(TagType::Anonymous, 0)?;
        tlvwriter.put_str8(TagType::Context, 1, &pB)?;
        tlvwriter.put_str8(TagType::Context, 2, &cB)?;
        tlvwriter.put_end_container()?;

        proto_ctx.session.set_exchange_data(
            proto_ctx.exch_id,
            ExchangeRole::Responder,
            spake2_boxed,
        )?;
        Ok(())
    }

    pub fn handle_pbkdfparamrequest(
        &mut self,
        proto_ctx: &mut ProtoCtx,
        tx_ctx: &mut TxCtx,
    ) -> Result<(), Error> {
        let (initiator_random, initiator_sessid, passcode_id, has_params) =
            extract_pbkdfreq_params(proto_ctx.buf)?;
        info!(
            "random: {:x?} sessid: {} passid: {} hasparams:{}",
            initiator_random, initiator_sessid, passcode_id, has_params
        );

        if passcode_id != 0 {
            error!("Can't yet handle passcode_id != 0");
            return Err(Error::Invalid);
        }

        let mut our_random: [u8; 32] = [0; 32];
        rand::thread_rng().fill_bytes(&mut our_random);

        let mut spake2p = Box::new(Spake2P::new());

        // Generate response
        let mut tlvwriter = TLVWriter::new(tx_ctx.get_write_buf());
        tlvwriter.put_start_struct(TagType::Anonymous, 0)?;
        tlvwriter.put_str8(TagType::Context, 1, initiator_random)?;
        tlvwriter.put_str8(TagType::Context, 2, &our_random)?;
        tlvwriter.put_u16(TagType::Context, 3, proto_ctx.session.get_local_sess_id())?;
        if !has_params {
            tlvwriter.put_start_struct(TagType::Context, 4)?;
            tlvwriter.put_u32(TagType::Context, 1, ITERATION_COUNT)?;
            tlvwriter.put_str8(TagType::Context, 2, &self.salt)?;
            tlvwriter.put_end_container()?;
        }
        tlvwriter.put_end_container()?;

        spake2p.set_context(proto_ctx.buf, tx_ctx.as_slice());
        proto_ctx
            .session
            .set_exchange_data(proto_ctx.exch_id, ExchangeRole::Responder, spake2p)?;
        Ok(())
    }
}

#[allow(non_snake_case)]
fn extract_pasepake1_params(buf: &[u8]) -> Result<&[u8], Error> {
    let root = get_root_node_struct(buf).ok_or(Error::InvalidData)?;
    let pA = root
        .find_element(1)
        .ok_or(Error::Invalid)?
        .get_slice()
        .ok_or(Error::Invalid)?;
    Ok(pA)
}

fn extract_pbkdfreq_params(buf: &[u8]) -> Result<(&[u8], u16, u16, bool), Error> {
    let root = get_root_node_struct(buf).ok_or(Error::InvalidData)?;
    let initiator_random = root
        .find_element(1)
        .ok_or(Error::Invalid)?
        .get_slice()
        .ok_or(Error::InvalidData)?;
    let initiator_sessid = root
        .find_element(2)
        .ok_or(Error::Invalid)?
        .get_u16()
        .ok_or(Error::Invalid)?;
    let passcode_id = root
        .find_element(3)
        .ok_or(Error::Invalid)?
        .get_u16()
        .ok_or(Error::Invalid)?;
    let has_params = root
        .find_element(4)
        .ok_or(Error::Invalid)?
        .get_bool()
        .ok_or(Error::Invalid)?;
    Ok((initiator_random, initiator_sessid, passcode_id, has_params))
}

const PBKDF_RANDOM_LEN: usize = 32;
#[derive(Default)]
pub struct PBKDFParamReq {
    pub initiator_random: [u8; PBKDF_RANDOM_LEN],
    pub initiator_sessid: u16,
    pub passcode_id: u16,
    pub has_params: bool,
}

impl PBKDFParamReq {
    pub fn new(
        initiator_random_ref: &[u8],
        initiator_sessid: u16,
        passcode_id: u16,
        has_params: bool,
    ) -> Option<Self> {
        if initiator_random_ref.len() != PBKDF_RANDOM_LEN {
            None
        } else {
            let mut req = PBKDFParamReq::default();
            req.initiator_random.copy_from_slice(initiator_random_ref);
            req.initiator_sessid = initiator_sessid;
            req.passcode_id = passcode_id;
            req.has_params = has_params;
            Some(req)
        }
    }
}
