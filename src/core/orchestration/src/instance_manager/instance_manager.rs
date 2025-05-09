use core::panic;
use std::{
    collections::{HashMap, VecDeque}, f32::consts::E, process::Command, sync::Arc, thread, time::{self, Instant}
};

use log::{debug, error, info, warn};
use mcore::hash256::HASH256;
use reqwest::header::CACHE_CONTROL;
use quorum_events::event::Event;
use quorum_network::types::message::NetMessage;
use quorum_proto::scheme_types::{Group, ThresholdScheme};
use quorum_protocols::{
    frost::protocol::FrostProtocol, interface::{ProtocolError, ThresholdRoundProtocol}, threshold_cipher::protocol::ThresholdCipherProtocol, threshold_coin::protocol::ThresholdCoinProtocol, threshold_signature::protocol::ThresholdSignatureProtocol
    // threshold_coin::protocol::ThresholdCoinProtocol,
    // threshold_signature::protocol::ThresholdSignatureProtocol,
};
use quorum_schemes::{
    dl_schemes::signatures::frost::FrostOptions, interface::{Ciphertext, SchemeError}, keys::{key_store::KeyEntry, keys::PrivateKeyShare}
};
use tokio::sync::{oneshot, Notify};
use tonic::{Code, Status};

use crate::{
    instance_manager::instance::{self, Instance},
    instance_manager::protocol_executor::ThresholdProtocolExecutor,
    interface::ThresholdProtocol,
    key_manager::key_manager::KeyManagerCommand,
};
/// Upper bound on the number of finished instances which to store.
const DEFAULT_INSTANCE_CACHE_SIZE: usize = 10000;
/// Number of instances which to look at when trying to find ones to eject.
const INSTANCE_CACHE_CLEANUP_SCAN_LENGTH: usize = 5000;

/// InstanceCache implements a first-in-first-out store for instances.
///
/// It is configured with an upper bound on the number of finished instances it will store. If a
/// new instance is added while at capacity, the oldest terminated instance is ejected.
///
/// Due to only evicting stored instances, there is no actual upper bound on its size. There could
/// well be an unbounded number of running instances.
struct InstanceCache {
    instance_data: HashMap<String, Instance>,
    capacity: usize,
    terminated_instances: VecDeque<String>,
}

impl InstanceCache {
    fn new(capacity: Option<usize>) -> InstanceCache {
        InstanceCache {
            instance_data: HashMap::new(),
            capacity: capacity.unwrap_or(DEFAULT_INSTANCE_CACHE_SIZE),
            terminated_instances: VecDeque::new(),
        }
    }

    fn get(&self, instance_id: &String) -> Option<&Instance> {
        self.instance_data.get(instance_id)
    }

    fn get_mut(&mut self, instance_id: &String) -> Option<&mut Instance> {
        self.instance_data.get_mut(instance_id)
    }

    fn insert(&mut self, instance_id: String, instance: Instance) {
        if self.instance_data.len() >= self.capacity {
            self.attempt_eject();
        }

        self.instance_data.insert(instance_id, instance);
    }

    /// Inform the instance cache that an instance has terminated, and is elligible for eviction if
    /// space is required.
    fn inform_of_termination(&mut self, instance_id: String) {

        if self.instance_data.contains_key(&instance_id) {
            info!("Instance ID {} terminated inserted in the terminated_instances queue", instance_id);
            let _instance = self.instance_data.get_mut(&instance_id).unwrap();
            let result = _instance.get_result().clone().unwrap(); // should be safe to unwrap here, result contains a value (result or error)
            self.terminated_instances.push_back(instance_id);
            if let Some(sender) = _instance.get_result_sender() {
                // we now need to send the result back to the caller
                if let Err(e) = sender.send(result) { // here we send the result back if there is a sync channel
                    error!("Error sending result to rpc_request_handler: {:?}", e);
                }
            } else {
                warn!("Error sending result to rpc_request_handler, sender is None -- maybe it was async request");
            }
        } else {
            error!(
                "Got informed that instance ID {} terminated, but no such instance was found",
                instance_id
            );
        }
    }

    /// Attempts to remove terminated instances from the store to get back to the desired capacity.
    /// Returns the number of ejected instances.
    fn attempt_eject(&mut self) -> usize {
        let mut current_iteration = 0;
        let max_iterations = {
            if self.terminated_instances.len() < INSTANCE_CACHE_CLEANUP_SCAN_LENGTH {
                self.terminated_instances.len()
            } else {
                INSTANCE_CACHE_CLEANUP_SCAN_LENGTH
            }
        };

        info!(
            "Cleaning up instance store by ejecting up to {} terminated instances.",
            max_iterations
        );

        // removed: && self.instance_data.len() > self.capacity
        // This condition is not necessary, as the loop will stop as soon as the cache is at capacity.
        while current_iteration < max_iterations {
            let candidate_id = self.terminated_instances.pop_front().unwrap();
            info!("Ejecting terminated instance {}", candidate_id);
            self.instance_data.remove(&candidate_id);

            current_iteration += 1;
        }

        info!(
            "Ejected {} instance(s) from instance store. Size (current / target): {} / {}",
            current_iteration,
            self.instance_data.len(),
            self.capacity
        );

        current_iteration
    }

    fn contains_key(&self, instance_id: &str) -> bool {
        self.instance_data.contains_key(instance_id)
    }
}

pub struct InstanceManager {
    key_manager_command_sender: tokio::sync::mpsc::Sender<KeyManagerCommand>,
    instance_command_receiver: tokio::sync::mpsc::Receiver<InstanceManagerCommand>,
    instance_command_sender: tokio::sync::mpsc::Sender<InstanceManagerCommand>,
    outgoing_p2p_sender: tokio::sync::mpsc::Sender<NetMessage>,
    incoming_p2p_receiver: tokio::sync::mpsc::Receiver<NetMessage>,
    instances: InstanceCache,
    backlog: HashMap<String, BacklogData>,
    backlog_interval: tokio::time::Interval,
    event_emitter_sender: tokio::sync::mpsc::Sender<Event>,
}

const BACKLOG_CHECK_INTERVAL: u64 = 60;

// BacklogData keeps all the messages that are destined for a specific instance,
// plus a field checked, which is used to detect too old backlog data.
struct BacklogData {
    messages: Vec<NetMessage>,
    checked: bool,
}

#[derive(Debug)]
pub enum StartInstanceRequest {
    Decryption {
        ciphertext: Ciphertext,
    },
    Signature {
        message: Vec<u8>,
        label: Vec<u8>,
        scheme: ThresholdScheme,
        group: Group,
        key_id: Option<String>,
    },
    Coin {
        name: Vec<u8>,
        scheme: ThresholdScheme,
        group: Group,
        key_id: Option<String>,
    },
}

// InstanceStatus describes the currenct state of a protocol instance.
// The field result has meaning only when finished == true.
#[derive(Debug, Clone)]
pub struct InstanceStatus {
    pub scheme: ThresholdScheme,
    pub group: Group,
    pub finished: bool,
    pub result: Option<Result<Vec<u8>, ProtocolError>>,
}

#[derive(Debug)]
pub enum InstanceManagerCommand {
    CreateInstance {
        request: StartInstanceRequest,
        responder: tokio::sync::oneshot::Sender<Result<String, ProtocolError>>,
    },

    CreateInstanceSync {
        request: StartInstanceRequest,
        responder: tokio::sync::oneshot::Sender<Result<String, ProtocolError>>,
        sync_responder: tokio::sync::oneshot::Sender<Result<Vec<u8>, ProtocolError>>,
    },

    GetInstanceStatus {
        instance_id: String,
        responder: tokio::sync::oneshot::Sender<Option<InstanceStatus>>,
    },

    StoreResult {
        instance_id: String,
        result: Result<Vec<u8>, ProtocolError>,
    },

    UpdateInstanceStatus {
        instance_id: String,
        status: String,
        error: Option<ProtocolError>,
    },
}

impl InstanceManager {
    pub fn new(
        key_manager_command_sender: tokio::sync::mpsc::Sender<KeyManagerCommand>,
        instance_command_receiver: tokio::sync::mpsc::Receiver<InstanceManagerCommand>,
        instance_command_sender: tokio::sync::mpsc::Sender<InstanceManagerCommand>,
        outgoing_p2p_sender: tokio::sync::mpsc::Sender<NetMessage>,
        incoming_p2p_receiver: tokio::sync::mpsc::Receiver<NetMessage>,
        event_emitter_sender: tokio::sync::mpsc::Sender<Event>,
    ) -> Self {
        return Self {
            key_manager_command_sender,
            instance_command_receiver,
            instance_command_sender,
            outgoing_p2p_sender,
            incoming_p2p_receiver,
            instances: InstanceCache::new(None),
            backlog: HashMap::new(),
            backlog_interval: tokio::time::interval(tokio::time::Duration::from_secs(
                BACKLOG_CHECK_INTERVAL as u64,
            )),
            event_emitter_sender,
        };
    }

    pub async fn run(&mut self, shutdown_notify: Arc<Notify>) -> Result<(), String> {
        loop {
            tokio::select! {
                _ = shutdown_notify.notified() => {
                    info!("Instance manager shutting down");
                    return Ok(());
                }
                command = self.instance_command_receiver.recv() => {
                    match command {
                        Some(cmd) => {
                            match cmd {
                                InstanceManagerCommand::CreateInstance{
                                        request,
                                        responder
                                    } => {
                                        let result = self.start(request, None).await;
                                        if result.is_err() {
                                            error!("Error starting instance: {:?}", result.unwrap_err());
                                        } else {
                                            if let Err(e) = responder.send(Ok(result.unwrap())) { // here we send back the instance id
                                                error!("Error sending response to instance creation request: {:?}", e);
                                                
                                            };
                                        }
                                    },

                                InstanceManagerCommand::CreateInstanceSync{
                                        request,
                                        responder,
                                        sync_responder
                                    } => {
                                        let result = self.start(request, Some(sync_responder)).await;
                                        if result.is_err() {
                                            let err = result.unwrap_err().clone();
                                            error!("Error starting instance: {:?}", err);
                                            if let Err(e) = responder.send(Err(err.clone())) { // here we send back a potential error at startup 
                                                error!("Error sending response to instance creation request: {:?}", e); 
                                            };
                                        } else {
                                            info!("Instance created successfully");
                                            if let Err(e) = responder.send(Ok(result.unwrap())) { // here we send back the instance id  
                                                error!("Error sending response to instance creation request: {:?}", e); 
                                            };
                                        }
                                },    

                                InstanceManagerCommand::GetInstanceStatus { instance_id, responder } => {
                                    let result = match self.instances.get(&instance_id) {
                                        Some(instance) => {

                                            Some(InstanceStatus {
                                                scheme: instance.get_scheme().clone(),
                                                group: instance.get_group().clone(),
                                                finished: instance.is_finished(),
                                                result: instance.get_result().clone()
                                            })
                                        },
                                        None => {
                                            None
                                        },
                                    };

                                    responder.send(result).expect("The receiver for responder in StateUpdateCommand::GetInstanceResult has been closed.");
                                },

                                InstanceManagerCommand::StoreResult {instance_id, result } => {
                                    let instance = self.instances.get_mut(&instance_id);

                                    match instance {
                                        Some(_instance) => {
                                            _instance.set_result(result.clone()); //save in cache
                                            self.instances.inform_of_termination(instance_id.clone()); // inform the cache that the instance has finished
                                        },
                                        None => error!("Instance {} not present in cache", instance_id)
                                    }
                                },

                                InstanceManagerCommand::UpdateInstanceStatus {instance_id, status, error } => {
                                    if error.is_some() {
                                        let instance = self.instances.get_mut(&instance_id);
                                        match instance {
                                            Some(_instance) => {
                                                _instance.set_status(&status);
                                                _instance.set_result(Err(error.unwrap()));
                                                self.instances.inform_of_termination(instance_id.clone());
                                            },
                                            None => error!("Error updating instance failed status for instance {}", instance_id)
                                        }
                                    } else {
                                        let instance = self.instances.get_mut(&instance_id);
                                        match instance {
                                            Some(_instance) => _instance.set_status(&status),
                                            None => error!("Error updating instance status for instance {}", instance_id)
                                        }
                                    }
                                }
                            }
                        },
                        None => {
                            warn!("Instance manager command channel closed. Shutting down.");
                            return Err("Instance manager command channel closed".to_string());
                        }
                    }
                }

                incoming_message = self.incoming_p2p_receiver.recv() => {
                    match incoming_message {
                        Some(net_message) => {
                            debug!("Received message from incoming channel: {:?}", net_message);
                            let instance =  self.instances.get(&net_message.get_instace_id());

                            // First check, if an instance already exists for that message
                            match instance {
                                Some(_instance) => {
                                    // If yes, forward the message to the instance. (ok if the following returns Err, it only means the instance has finished in the mean time)
                                    if !(_instance.is_finished()){
                                        let _ =  _instance.send_message(net_message).await;
                                    }
                                },
                                None => {
                                    let instance_id = net_message.get_instace_id().clone();
                                    // Otherwise, backlog the message. This can happen for two reasons:
                                    // - The instance has already finished and the corresponding sender has been removed from the instance_senders.
                                    // - The instance has not yet started because the corresponding request has not yet arrived.
                                    // In both cases, we backlog the message. If the instance has already been finished,
                                    // the backlog will be deleted after at most 2*BACKLOG_CHECK_INTERVAL seconds
                                    info!(
                                        "Backlogging message for instance with id: {:?}",
                                        &instance_id
                                    );
                                    if let Some(backlog_data) =  self.backlog.get_mut(&instance_id) {
                                        backlog_data.messages.push(net_message);
                                    } else {
                                        let mut backlog_data = BacklogData{ messages: Vec::new(), checked: false };
                                        backlog_data.messages.push(net_message);
                                        self.backlog.insert(instance_id, backlog_data);
                                    }
                                }
                            }
                        },
                        None => {
                            warn!("Incoming message channel closed");
                            return Err("Incoming message channel closed".to_string());
                        }
                    }
                }

                // Detect and delete too old backlog data, so the backlogged_instances field does not grow forever.
                // We assume that an instance will be started at most BACKLOG_CHECK_INTERVAL seconds
                // after a message for that instance has been received. Otherwise, it will never start, so we can delete backlogged messages.
                // Every BACKLOG_CHECK_INTERVAL seconds, go through all backlogged_instances.
                // If the field 'checked' is true, delete the backlogged instance. If it is false, set it to true.
                // This ensures that, if a backlogged instance gets deleted, then it has been waiting for at least BACKLOG_CHECK_INTERVAL seconds.
                _ = self.backlog_interval.tick() => {
                    self.backlog.retain(|_, v| v.checked == false);
                    for (_, v) in self.backlog.iter_mut(){
                        v.checked = true;
                    }
                    info!("Old backlogged instances deleted");

                    // also clean the cache every minute
                    // self.instances.attempt_eject();
                }
            }
        }
    }

    pub async fn start<'a>(
        &mut self,
        instance_request: StartInstanceRequest,
        mut sync_responder: Option<tokio::sync::oneshot::Sender<Result<Vec<u8>, ProtocolError>>>,
        
    ) -> Result<String, ProtocolError> {
        // Create a unique instance_id for this instance
        let instance_id = assign_instance_id(&instance_request);

        match instance_request {
            StartInstanceRequest::Decryption { ciphertext } => {
                let now = Instant::now();
                let key = self
                    .setup_instance(
                        ciphertext.get_scheme(),
                        ciphertext.get_group(),
                        &instance_id,
                        Some(ciphertext.get_key_id().to_string()),
                    )
                    .await;

                debug!("Set up instance after {}ms", now.elapsed().as_millis());

                if key.is_err() {
                    let e = key.unwrap_err();
                    if e.code() == Code::AlreadyExists {
                        return Ok(instance_id);
                    }
                    error!("Key not found");
                    return Err(ProtocolError::SchemeError(SchemeError::Aborted(String::from("key not found"))));
                }

                let key = key.unwrap();

                let (sender, receiver) = tokio::sync::mpsc::channel::<NetMessage>(32);

                let instance = Instance::new(
                    instance_id.clone(),
                    ciphertext.get_scheme(),
                    ciphertext.get_group().clone(),
                    Some(sender),
                    sync_responder.take(),
                );

                // Create the new protocol instance
                let prot =
                    ThresholdCipherProtocol::new(key.clone(), ciphertext, instance_id.clone());

                let executor = ThresholdProtocolExecutor::new(
                    receiver,
                    self.outgoing_p2p_sender.clone(),
                    instance_id.clone(),
                    self.event_emitter_sender.clone(),
                    prot,
                );

                self.instances.insert(instance_id.clone(), instance);

                let sender = self.instance_command_sender.clone();
                let id = instance_id.clone();

                // Start it in a new thread, so that the client does not block until the protocol is finished.
                tokio::spawn(async move {
                    let result = Self::execute_protocol(
                        executor,
                        id,
                        sender,
                    ).await;
                    if result.is_err() {
                        let error = result.unwrap_err();
                        error!("Error during protocol execution protocol: {:?}", error);  
                    }
                });

                // Maybe here handle error situations
                _ = self.forward_backlogged_messages(instance_id.clone());

                debug!(
                    "Set up instance thread after {}ms",
                    now.elapsed().as_millis()
                );

                return Ok(instance_id.clone());
            }
            StartInstanceRequest::Signature {
                message,
                label,
                scheme,
                group,
                key_id,
            } => {
                let key = self
                    .setup_instance(scheme, &group, &instance_id, key_id)
                    .await;

                if key.is_err() {
                    let e = key.unwrap_err();
                    if e.code() == Code::AlreadyExists {
                        return Ok(instance_id);
                    }
                    error!("Key not found");
                    return Err(ProtocolError::SchemeError(SchemeError::Aborted(String::from("key not found"))));
                }

                let key = key.unwrap();

                let (sender, receiver) = tokio::sync::mpsc::channel::<NetMessage>(32);

                let instance = Instance::new(
                    instance_id.clone(), 
                    scheme, 
                    group.clone(), 
                    Some(sender), 
                    sync_responder.take()
                );

                match scheme {
                    ThresholdScheme::Frost => {
                        let prot = FrostProtocol::new(
                            key, 
                            &message, 
                            b"label",
                            FrostOptions::NoPrecomputation,
                            Option::None
                        );
                        let executor = ThresholdProtocolExecutor::new(
                            receiver,
                            self.outgoing_p2p_sender.clone(),
                            instance_id.clone(),
                            self.event_emitter_sender.clone(),
                            prot,
                        );
                        self.instances.insert(instance_id.clone(), instance);

                        let sender = self.instance_command_sender.clone();
                        let id = instance_id.clone();

                        // Start it in a new thread, so that the client does not block until the protocol is finished.
                        tokio::spawn(async move {
                            let result = Self::execute_protocol(
                                executor,
                                id,
                                sender,
                            ).await;
                            if result.is_err() {
                                error!("Error starting protocol: {:?}", result.unwrap_err());
                            }
                        });

                        _ = self.forward_backlogged_messages(instance_id.clone());
                
                        return Ok(instance_id.clone());
                    },
                    _ => {
                        let prot = ThresholdSignatureProtocol::new(key,Some(&message),&label);
                        let executor = ThresholdProtocolExecutor::new(
                            receiver,
                            self.outgoing_p2p_sender.clone(),
                            instance_id.clone(),
                            self.event_emitter_sender.clone(),
                            prot,
                        );
                        self.instances.insert(instance_id.clone(), instance);

                        let sender = self.instance_command_sender.clone();
                        let id = instance_id.clone();

                        // Start it in a new thread, so that the client does not block until the protocol is finished.
                        tokio::spawn(async move {
                            let result = Self::execute_protocol(
                                executor,
                                id,
                                sender,
                            ).await;
                            if result.is_err() {
                                error!("Error starting protocol: {:?}", result.unwrap_err());
                            }
                        });

                        _ = self.forward_backlogged_messages(instance_id.clone());
        
                        return Ok(instance_id.clone());
                    },
                };
                
            }
            StartInstanceRequest::Coin {
                name,
                scheme,
                group,
                key_id,
            } => {
                let key = self
                    .setup_instance(scheme, &group, &instance_id, key_id)
                    .await;

                if key.is_err() {
                    let e = key.unwrap_err();
                    if e.code() == Code::AlreadyExists {
                        return Ok(instance_id);
                    }
                    error!("Key not found");
                    return Err(ProtocolError::SchemeError(SchemeError::Aborted(String::from("key not found"))));
                }

                let key = key.unwrap();

                let (sender, receiver) = tokio::sync::mpsc::channel::<NetMessage>(32);

                let instance = Instance::new(
                    instance_id.clone(), 
                    scheme, 
                    group, 
                    Some(sender),
                    sync_responder.take(),
                    );

                // Create the new protocol instance
                let prot = ThresholdCoinProtocol::new(
                    key,
                    &name,
                );

                let executor = ThresholdProtocolExecutor::new(
                    receiver,
                    self.outgoing_p2p_sender.clone(),
                    instance_id.clone(),
                    self.event_emitter_sender.clone(),
                    prot,
                );

                self.instances.insert(instance_id.clone(), instance);

                let sender = self.instance_command_sender.clone();
                let id = instance_id.clone();

                // Start it in a new thread, so that the client does not block until the protocol is finished.
                tokio::spawn(async move {
                    let result = Self::execute_protocol(
                        executor,
                        id,
                        sender,
                    ).await;
                    if result.is_err() {
                        error!("Error starting protocol: {:?}", result.unwrap_err());
                    }
                });

                _ = self.forward_backlogged_messages(instance_id.clone());
                
                return Ok(instance_id.clone());
            }
        }
    }

    async fn execute_protocol(
       
        mut executor: (impl ThresholdProtocol + std::marker::Send + 'static),
        instance_id: String,
        sender: tokio::sync::mpsc::Sender<InstanceManagerCommand>,
    ) -> Result<(), ProtocolError> {
        
            let result = executor.run().await;
            let instance_manager_sender = sender.clone();

            match result {
                Ok(_) => {
                    // Protocol terminated, update state with the result.
                    info!("Instance {:?} finished", instance_id.clone());
                },
                Err(e) => {
                    error!("Error running protocol: {:?}", e);

                    // Inform the instance manager about the error and the failed status. 
                    instance_manager_sender.send(InstanceManagerCommand::UpdateInstanceStatus {
                        instance_id: instance_id,
                        status: "Failed".to_string(),
                        error: Some(e.clone()),
                    }).await.expect("channel closed by instance manager");

                
                    return Err(e);
                }
            }
            
            if sender
                .send(InstanceManagerCommand::StoreResult {
                    instance_id: instance_id.clone(),
                    result: result.clone(),
                })
                .await
                .is_err()
                {
                // loop until transmission successful
                //COMMENT_R: can this loop occupy the CPU?
                error!("Error storing result, channel closed");
                return Err(ProtocolError::InternalError);
                // thread::sleep(time::Duration::from_millis(500)); // wait for 500ms before trying again
            }

            drop(sender); // close the sender channel

            Ok(())
    }

    fn forward_backlogged_messages(&mut self, instance_id: String) {
        let instance = self.instances.get(&instance_id);

        if instance.is_none() {
            return;
        }
        let instance = instance.unwrap();

        let backlog = self.backlog.get(&instance_id);
        if backlog.is_none() {
            return;
        }

        let backlog = backlog.unwrap();
        let messages = backlog.messages.clone();
        if let Some(sender) = instance.get_sender(){
            let instance_id_cloned = instance_id.clone();
            tokio::spawn(async move {
                for msg in &messages {
                    let _ = sender.send(msg.clone()).await;
                }
                info!(
                    "All the messages backlogged for instance {:?} have been sent",
                    instance_id_cloned
                );
                drop(sender);
            });
        }
        // if the sender is back to None it means the instance has finished
        self.backlog.remove(&instance_id);
    }

    async fn get_key_by_id(&self, key_id: &str) -> Result<Arc<KeyEntry>, String> {
        let (response_sender, response_receiver) =
            oneshot::channel::<Result<Arc<KeyEntry>, String>>();

        if self
            .key_manager_command_sender
            .send(KeyManagerCommand::GetKeyById {
                id: String::from(key_id),
                responder: response_sender,
            })
            .await
            .is_err()
        {
            return Err(String::from("Could not contact key manager"));
        }

        let response = response_receiver.await;
        if response.is_ok() {
            return response.unwrap();
        }

        return Err(String::from("Got no response from key manager"));
    }

    async fn get_key_by_scheme_and_group(
        &self,
        scheme: &ThresholdScheme,
        group: &Group,
    ) -> Result<Arc<KeyEntry>, String> {
        let (response_sender, response_receiver) =
            oneshot::channel::<Result<Arc<KeyEntry>, String>>();

        if self
            .key_manager_command_sender
            .send(KeyManagerCommand::GetKeyBySchemeAndGroup {
                scheme: scheme.clone(),
                group: group.clone(),
                responder: response_sender,
            })
            .await
            .is_err()
        {
            return Err(String::from("Could not contact key manager"));
        }

        let response = response_receiver.await;
        if response.is_ok() {
            return response.unwrap();
        }

        return Err(String::from("Got no response from key manager"));
    }

    async fn setup_instance<'a>(
        &self,
        scheme: ThresholdScheme,
        group: &Group,
        instance_id: &str,
        key_id: Option<String>,
    ) -> Result<Arc<PrivateKeyShare>, Status> {
        if self.instances.contains_key(instance_id) {
            error!(
                "A request with the same id '{:?}' already exists.",
                instance_id
            );
            return Err(Status::new(
                Code::AlreadyExists,
                format!("A similar request with request_id {instance_id} already exists."),
            ));
        }

        // Retrieve private key for this instance
        let key: Arc<KeyEntry>;
        if let Some(kid) = key_id {
            let key_result = self.get_key_by_id(&kid).await;

            match key_result {
                Ok(key_entry) => key = key_entry,
                Err(err) => return Err(Status::new(Code::InvalidArgument, err)),
            };
        } else {
            let key_result = self.get_key_by_scheme_and_group(&scheme, group).await;

            match key_result {
                Ok(key_entry) => key = key_entry,
                Err(err) => return Err(Status::new(Code::InvalidArgument, err)),
            };
        };
        info!(
            "Using key with id: {:?} for request {:?}",
            key.id, &instance_id
        );

        let key = key.sk.clone();

        if key.is_none() {
            return Err(Status::new(Code::InvalidArgument, "private key not found"));
        }
        let key = Arc::new(key.unwrap());

        Ok(key)
    }
}

/* TODO: rethink how to assign instance ids */
fn assign_instance_id(request: &StartInstanceRequest) -> String {
    let mut digest = HASH256::new();

    match request {
        StartInstanceRequest::Decryption { ciphertext } => {
            digest.process_array(ciphertext.get_ck());
            let h: &[u8] = &digest.hash();
            return hex::encode(h);
        }
        StartInstanceRequest::Signature {
            message: _,
            label,
            scheme,
            group,
            key_id,
        } => {
            digest.process_array(&label);
            digest.process_array(scheme.as_str_name().as_bytes());
            digest.process_array(group.as_str_name().as_bytes());

            if key_id.is_some() {
                digest.process_array(key_id.clone().unwrap().as_bytes())
            }

            let h: &[u8] = &digest.hash();
            return hex::encode(h);
        }
        StartInstanceRequest::Coin {
            name,
            scheme: _,
            group: _,
            key_id,
        } => {
            /* PROBLEM: Hashing the whole
            name might become a bottleneck for long names */
            digest.process_array(&name);
            if key_id.is_some() {
                digest.process_array(key_id.clone().unwrap().as_bytes())
            }
            let h: &[u8] = &digest.hash();
            return hex::encode(h);
        }
    }
}
