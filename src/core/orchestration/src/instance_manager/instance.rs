use core::fmt;
use clap::error;
use quorum_proto::scheme_types::{Group, ThresholdScheme};
use quorum_protocols::interface::ProtocolError;
use tokio::sync::mpsc::error::SendError;
use quorum_network::types::message::NetMessage;
use log::error;

pub struct Instance {
    id: String,
    scheme: ThresholdScheme,
    group: Group,
    message_channel_sender: Option<tokio::sync::mpsc::Sender<NetMessage>>,
    status: String,
    finished: bool,
    result: Option<Result<Vec<u8>, ProtocolError>>,
    result_sender: Option<tokio::sync::oneshot::Sender<Result<Vec<u8>, ProtocolError>>>,
}

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Instance {{\nid:{}\n, scheme:{}\ngroup:{}\nstatus:{}\n }}",
            self.id,
            self.scheme.as_str_name(),
            self.group.as_str_name(),
            &self.status
        )
    }
}

impl Instance {
    pub fn new(
        id: String,
        scheme: ThresholdScheme,
        group: Group,
        message_channel_sender: Option<tokio::sync::mpsc::Sender<NetMessage>>,
        result_sender: Option<tokio::sync::oneshot::Sender<Result<Vec<u8>, ProtocolError>>>,
    ) -> Self {
        return Self {
            id,
            scheme,
            group,
            message_channel_sender,
            result_sender,
            status: String::from("created"),
            finished: false,
            result: Option::None,
        };
    }

    pub fn set_status(&mut self, status: &str) {
        self.status = String::from(status);
    }

    pub fn is_finished(&self) -> bool {
        return self.finished;
    }

    pub fn get_result(&self) -> &Option<Result<Vec<u8>, ProtocolError>> {
        return &self.result;
    }

    pub fn set_result(&mut self, result: Result<Vec<u8>, ProtocolError>) {
        self.result = Some(result);
        self.finished = true;
        if let Some(sender) = self.message_channel_sender.take(){
            drop(sender);
        }
    }

    pub fn get_scheme(&self) -> ThresholdScheme {
        self.scheme.clone()
    }

    pub fn get_group(&self) -> Group {
        self.group.clone()
    }

    pub fn get_result_sender(&mut self) -> Option<tokio::sync::oneshot::Sender<Result<Vec<u8>, ProtocolError>>> {
        return self.result_sender.take(); // take puts None in the fields ... this means that the sender can be consumed only once. It fits with the one-shot channel
    }

    pub async fn send_message(&self, message: NetMessage) -> Result<(), SendError<NetMessage>> {
        if let Some(sender) = &self.message_channel_sender {
            return sender.send(message).await
        }
        error!("Trying to send message to finished instance");
        return Err(SendError(message));
    }

    pub fn get_sender(&self) -> Option<tokio::sync::mpsc::Sender<NetMessage>>{
        self.message_channel_sender.clone()
    }
}
