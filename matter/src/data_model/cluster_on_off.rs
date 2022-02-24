use super::objects::*;
use crate::{
    cmd_enter,
    error::*,
    interaction_model::{command::CommandReq, core::IMStatusCode},
    tlv_common::TagType,
    tlv_writer::TLVWriter,
};
use log::info;

const CLUSTER_ONOFF_ID: u32 = 0x0006;

const ATTR_ON_OFF_ID: u16 = 0x0;

const CMD_OFF_ID: u16 = 0x00;
const CMD_ON_ID: u16 = 0x01;
const CMD_TOGGLE_ID: u16 = 0x02;

fn attr_on_off_new() -> Result<Box<Attribute>, Error> {
    // Id: 0, Value: false
    Attribute::new(ATTR_ON_OFF_ID, AttrValue::Bool(false))
}

pub struct OnOffCluster {
    base: Cluster,
}

impl OnOffCluster {
    pub fn new() -> Result<Box<Self>, Error> {
        let mut cluster = Box::new(OnOffCluster {
            base: Cluster::new(CLUSTER_ONOFF_ID),
        });
        cluster.base.add_attribute(attr_on_off_new()?)?;
        Ok(cluster)
    }
}

impl ClusterType for OnOffCluster {
    fn base(&self) -> &Cluster {
        &self.base
    }
    fn base_mut(&mut self) -> &mut Cluster {
        &mut self.base
    }

    fn read_attribute(&self, tag: TagType, tw: &mut TLVWriter, attr_id: u16) -> Result<(), Error> {
        self.base.read_attribute(tag, tw, attr_id)
    }

    fn handle_command(&mut self, cmd_req: &mut CommandReq) -> Result<(), IMStatusCode> {
        let cmd = cmd_req.cmd.path.leaf.map(|a| a as u16);
        println!("Received command: {:?}", cmd);

        match cmd {
            Some(CMD_OFF_ID) => {
                cmd_enter!("Off");
                cmd_req.trans.complete();
                Err(IMStatusCode::Sucess)
            }
            Some(CMD_ON_ID) => {
                cmd_enter!("On");
                cmd_req.trans.complete();
                Err(IMStatusCode::Sucess)
            }
            Some(CMD_TOGGLE_ID) => {
                cmd_enter!("Toggle");
                cmd_req.trans.complete();
                Err(IMStatusCode::Sucess)
            }
            _ => Err(IMStatusCode::UnsupportedCommand),
        }
    }
}
