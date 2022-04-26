use num_derive::FromPrimitive;

use crate::{error::Error, transport::packet::Packet};

use super::status_report::{create_status_report, GeneralCode};

/* Interaction Model ID as per the Matter Spec */
pub const PROTO_ID_SECURE_CHANNEL: usize = 0x00;

#[derive(FromPrimitive, Debug)]
pub enum OpCode {
    MsgCounterSyncReq = 0x00,
    MsgCounterSyncResp = 0x01,
    MRPStandAloneAck = 0x10,
    PBKDFParamRequest = 0x20,
    PBKDFParamResponse = 0x21,
    PASEPake1 = 0x22,
    PASEPake2 = 0x23,
    PASEPake3 = 0x24,
    CASESigma1 = 0x30,
    CASESigma2 = 0x31,
    CASESigma3 = 0x32,
    CASESigma2Resume = 0x33,
    StatusReport = 0x40,
}

#[derive(PartialEq)]
pub enum SCStatusCodes {
    SessionEstablishmentSuccess = 0,
    NoSharedTrustRoots = 1,
    InvalidParameter = 2,
    CloseSession = 3,
    Busy = 4,
    SessionNotFound = 5,
}

pub fn create_sc_status_report(
    proto_tx: &mut Packet,
    status_code: SCStatusCodes,
    proto_data: Option<&[u8]>,
) -> Result<(), Error> {
    let general_code = match status_code {
        SCStatusCodes::SessionEstablishmentSuccess | SCStatusCodes::CloseSession => {
            GeneralCode::Success
        }
        SCStatusCodes::Busy
        | SCStatusCodes::InvalidParameter
        | SCStatusCodes::NoSharedTrustRoots
        | SCStatusCodes::SessionNotFound => GeneralCode::Failure,
    };
    create_status_report(
        proto_tx,
        general_code,
        PROTO_ID_SECURE_CHANNEL as u32,
        status_code as u16,
        proto_data,
    )
}

pub fn create_mrp_standalone_ack(proto_tx: &mut Packet) {
    proto_tx.set_proto_id(PROTO_ID_SECURE_CHANNEL as u16);
    proto_tx.set_proto_opcode(OpCode::MRPStandAloneAck as u8);
    proto_tx.unset_reliable();
}
