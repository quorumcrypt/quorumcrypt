use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use log::{error, info, warn};
use quorum_events::event::Event;
use quorum_network::types::message::NetMessage;
use quorum_schemes::interface::{
    SchemeError, Serializable, Signature,
    SignatureShare, ThresholdScheme, ThresholdSignature, ThresholdSignatureParams,
};
use quorum_schemes::keys::keys::PrivateKeyShare;
use quorum_schemes::scheme_types_impl::SchemeDetails;
use tonic::async_trait;

use crate::interface::{ProtocolError, ThresholdRoundProtocol};

use super::message_types::SignatureMessage;

pub struct ThresholdSignatureProtocol {
    private_key: Arc<PrivateKeyShare>,
    message: Option<Vec<u8>>,
    label: Vec<u8>,
    valid_shares: Vec<SignatureShare>,
    finished: bool,
    signature: Option<Signature>,
    received_share_ids: HashSet<u16>,
    // options: Option<ThresholdSignatureOptions>, //TODO: to review and remove
}

impl ThresholdRoundProtocol<NetMessage> for ThresholdSignatureProtocol {
    type ProtocolMessage = SignatureMessage;

    fn do_round(&mut self) -> Result<Self::ProtocolMessage, ProtocolError> {
        // compute and send decryption share
        let mut params = ThresholdSignatureParams::new();
        let share = ThresholdSignature::partial_sign(
            &(&self.message).clone().unwrap(),
            &self.label,
            &self.private_key,
            &mut params,
        )?;

        self.received_share_ids.insert(share.get_id());
        self.valid_shares.push(share.clone());
        Ok(SignatureMessage::ShareMessage(share))
    }

    fn is_ready_for_next_round(&self) -> bool {
        self.valid_shares.len() >= self.private_key.get_threshold() as usize
    }

    fn is_ready_to_finalize(&self) -> bool {
        self.valid_shares.len() >= self.private_key.get_threshold() as usize
    }

    fn finalize(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let sig = ThresholdSignature::assemble(
            &self.valid_shares,
            &(&self.message).clone().unwrap(),
            &self.private_key.get_public_key(),
        )?;
        self.signature = Option::Some(sig.clone());
        self.finished = true;
        info!("Issued a signature share.");
        return Ok(sig.to_bytes().unwrap()); //TODO: see if we need to handle unwrap(). It should be good because we just produced the sig
    }

    fn update(&mut self, message: Self::ProtocolMessage) -> Result<(), ProtocolError> {


        match message {
            SignatureMessage::ShareMessage(share) => {
                info!(
                    "Received share with id {:?}.",
                    share.get_id()
                );
                if self.finished {
                    return Ok(());
                }
        
                if self.received_share_ids.contains(&share.get_id()) {
                    warn!(
                        "Found share to be DUPLICATE with id {:?}. Share will be ignored.",
                        share.get_id()
                    );
                    return Ok(());
                }
                self.received_share_ids.insert(share.get_id());

                let verification_result = ThresholdSignature::verify_share(
                    &share,
                    &(&self.message).clone().unwrap(),
                    &self.private_key.get_public_key(),
                );
                match verification_result {
                    Ok(is_valid) => {
                        if !is_valid {
                            warn!("Received INVALID share with share_id: {:?}. Share will be ingored.", share.get_id());
                            return Ok(()); //TODO: maybe to hadle better, count how many we esclude
                        }
                    }
                    Err(err) => {
                        warn!("Encountered error when validating share with share_id: {:?}. Error:{:?}. Share will be ingored.",err, share.get_id());
                        return Ok(());
                    }
                }
        
                self.valid_shares.push(share);
                return Ok(());
            },
            _ => todo!() //default, unreachable
        }
    }
}


//TODO: to review and remove
// pub struct ThresholdSignaturePrecomputation {
//     private_key: Arc<PrivateKeyShare>,
//     chan_in: tokio::sync::mpsc::Receiver<Vec<u8>>,
//     chan_out: tokio::sync::mpsc::Sender<NetMessage>,
//     instance_id: String,
//     instance: InteractiveThresholdSignature,
// }

// #[async_trait]
// impl ThresholdProtocol for ThresholdSignatureProtocol {
//     async fn terminate(&mut self) -> Result<(), ProtocolError>{
//         todo!()
//     }
//     async fn run(&mut self) -> Result<Vec<u8>, ProtocolError> {
//         info!(
//             "<{:?}>: Starting threshold signature instance",
//             &self.instance_id
//         );

//         let event = Event::StartedInstance {
//             timestamp: Utc::now(),
//             instance_id: self.instance_id.clone(),
//         };

//         self.event_emitter_sender.send(event).await.unwrap();

//         if !self.precompute && self.instance.is_some() {
//             let _ = self
//                 .instance
//                 .as_mut()
//                 .unwrap()
//                 .set_msg(&(&self.message).clone().unwrap());
//         }

//         self.on_init().await?;

//         loop {
//             match self.chan_in.recv().await {
//                 Some(msg) => {
//                     if self.private_key.get_scheme().is_interactive() {
//                         match RoundResult::from_bytes(&msg) {
//                             Ok(round_result) => {
//                                 if self
//                                     .instance
//                                     .as_mut()
//                                     .unwrap()
//                                     .update(&round_result)
//                                     .is_err()
//                                 {
//                                     warn!(
//                                         "<{:?}>: Could not process round result. Will be ignored.",
//                                         &self.instance_id
//                                     );
//                                 }

//                                 if self.instance.as_mut().unwrap().is_ready_for_next_round() {
//                                     if self.instance.as_ref().unwrap().is_finished() {
//                                         self.finished = true;
//                                         let sig =
//                                             self.instance.as_mut().unwrap().get_signature()?;
//                                         self.signature = Some(sig);
//                                         self.terminate().await?;

//                                         info!("<{:?}>: Calculated signature.", &self.instance_id);

//                                         let result = self.signature.as_ref().unwrap().to_bytes();
//                                         if result.is_err() {
//                                             return Err(ProtocolError::SchemeError(
//                                                 result.unwrap_err(),
//                                             ));
//                                         }

//                                         let event = Event::FinishedInstance {
//                                             timestamp: Utc::now(),
//                                             instance_id: self.instance_id.clone(),
//                                         };
//                                         self.event_emitter_sender.send(event).await.unwrap();

//                                         return Ok(result.unwrap());
//                                     }

//                                     let rr = self.instance.as_mut().unwrap().do_round();
//                                     self.received_share_ids.clear();

//                                     if rr.is_err() {
//                                         error!(
//                                             "<{:?}>: Error while doing signature protocol round: {}",
//                                             &self.instance_id,
//                                             rr.unwrap_err().to_string()
//                                         );
//                                     } else {
//                                         let rr = rr.unwrap();
//                                         let _ = self.instance.as_mut().unwrap().update(&rr);

//                                         let message = NetMessage {
//                                             instance_id: self.instance_id.clone(),
//                                             message_data: rr.to_bytes().unwrap(),
//                                             is_total_order: false,
//                                         };
//                                         self.chan_out.send(message).await.unwrap();
//                                     }
//                                 }
//                             }
//                             Err(_) => {
//                                 warn!(
//                                     "<{:?}>: Could not deserialize round result. Round result will be ignored.",
//                                     &self.instance_id
//                                 );
//                                 continue;
//                             }
//                         }
//                     } else {
//                         match SignatureShare::from_bytes(&msg) {
//                             Ok(deserialized_share) => {
//                                 self.on_receive_signature_share(deserialized_share)?;
//                                 if self.finished {
//                                     self.terminate().await?;

//                                     let result = self.signature.as_ref().unwrap().to_bytes();
//                                     if result.is_err() {
//                                         return Err(ProtocolError::SchemeError(
//                                             result.unwrap_err(),
//                                         ));
//                                     }

//                                     let event = Event::FinishedInstance {
//                                         timestamp: Utc::now(),
//                                         instance_id: self.instance_id.clone(),
//                                     };
//                                     self.event_emitter_sender.send(event).await.unwrap();
//                                     return Ok(result.unwrap());
//                                 }
//                             }
//                             Err(_) => {
//                                 warn!(
//                                     "<{:?}>: Could not deserialize share. Share will be ignored.",
//                                     &self.instance_id
//                                 );
//                                 continue;
//                             }
//                         };
//                     }
//                 }
//                 None => {
//                     error!(
//                         "<{:?}>: Sender end unexpectedly closed. Protocol instance will quit.",
//                         &self.instance_id
//                     );
//                     self.terminate().await?;
//                     return Err(ProtocolError::InternalError);
//                 }
//             }
//         }
//         // todo: Currently the protocol instance will exist until it receives enough shares. Implement a timeout logic and exit the thread on expire.
//     }
// }

impl<'a> ThresholdSignatureProtocol {
    pub fn new(
        private_key: Arc<PrivateKeyShare>,
        message: Option<&Vec<u8>>,
        label: &Vec<u8>,
    ) -> Self {
        //TODO: to review and remove
        // let mut instance = Option::None;
        // if private_key.get_scheme().is_interactive() {
        //     info!("<{:?}>: Creating interactive instance", instance_id);
        //     let i = InteractiveThresholdSignature::new(&private_key);
        //     if i.is_err() {
        //         panic!("Error creating signature instance");
        //     }

        //     let mut i = i.unwrap();
        //     let _ = i.set_label(&label);

        //     instance = Option::Some(i);
        // }

        ThresholdSignatureProtocol {
            private_key,
            message: message.clone().cloned(),
            label: label.clone(),
            valid_shares: Vec::new(),
            finished: false,
            signature: Option::None,
            received_share_ids: HashSet::new(),
        }
    }
    
    //TODO: to review and remove
    // pub fn from_instance(
    //     instance: &InteractiveThresholdSignature,
    //     private_key: Arc<PrivateKeyShare>,
    //     message: &Vec<u8>,
    //     label: &Vec<u8>,
    //     chan_in: tokio::sync::mpsc::Receiver<Vec<u8>>,
    //     chan_out: tokio::sync::mpsc::Sender<NetMessage>,
    //     event_emitter_sender: tokio::sync::mpsc::Sender<Event>,
    //     instance_id: String,
    // ) -> Self {
    //     return ThresholdSignatureProtocol {
    //         private_key,
    //         message: Option::Some(message.clone()),
    //         label: label.clone(),
    //         chan_in,
    //         chan_out,
    //         instance_id,
    //         valid_shares: Vec::new(),
    //         finished: false,
    //         signature: Option::None,
    //         received_share_ids: HashSet::new(),
    //         instance: Option::Some(instance.clone()),
    //         precompute: true,
    //         event_emitter_sender,
    //     };
    // }
}


//TODO: Check how the precomputations work now
// impl<'a> ThresholdSignaturePrecomputation {
//     pub fn new(
//         private_key: Arc<PrivateKeyShare>,
//         label: &Vec<u8>,
//         chan_in: tokio::sync::mpsc::Receiver<Vec<u8>>,
//         chan_out: tokio::sync::mpsc::Sender<NetMessage>,
//         instance_id: String,
//     ) -> Self {
//         info!("<{:?}>: Creating precomputation instance", &instance_id);
//         let i = InteractiveThresholdSignature::new(&private_key);
//         if i.is_err() {
//             panic!("Error creating precomputation instance");
//         }

//         let instance = i.unwrap();

//         ThresholdSignaturePrecomputation {
//             private_key,
//             chan_in,
//             chan_out,
//             instance_id,
//             instance,
//         }
//     }

//     pub async fn run(&mut self) -> Result<InteractiveThresholdSignature, ProtocolError> {
//         if self.private_key.get_scheme() != ThresholdScheme::Frost {
//             error!(
//                 "<{:?}>: trying to use precompute on scheme other than Frost",
//                 &self.instance_id
//             );
//             return Err(ProtocolError::SchemeError(SchemeError::WrongScheme));
//         }

//         info!("<{:?}>: Instance starting.", &self.instance_id);

//         let rr = self.instance.do_round()?;
//         let message = NetMessage {
//             instance_id: self.instance_id.clone(),
//             message_data: rr.to_bytes().unwrap(),
//             is_total_order: false,
//         };

//         self.instance
//             .update(&rr)
//             .expect("Error processing round result");

//         let res = self.chan_out.send(message).await;

//         if res.is_ok() {
//             info!("<{:?}>: sent round result.", &self.instance_id);
//         } else {
//             error!("<{:?}>: error sending round result.", &self.instance_id);
//         }

//         loop {
//             match self.chan_in.recv().await {
//                 Some(msg) => {
//                     if self.private_key.get_scheme() == ThresholdScheme::Frost {
//                         match RoundResult::from_bytes(&msg) {
//                             Ok(round_result) => {
//                                 info!(
//                                     "<{:?}>: Precomputation round result received",
//                                     &self.instance_id
//                                 );
//                                 if self.instance.update(&round_result).is_err() {
//                                     warn!(
//                                         "<{:?}>: Could not process round result. Will be ignored.",
//                                         &self.instance_id
//                                     );
//                                 }

//                                 if self.instance.is_ready_for_next_round() {
//                                     info!("<{:?}>: Finished precomputation", &self.instance_id);
//                                     return Result::Ok(self.instance.clone()); // we have enough round results for round two - stop precomputation
//                                 }
//                             }
//                             Err(_) => {
//                                 warn!(
//                                     "<{:?}>: Could not deserialize round result. Round result will be ignored.",
//                                     &self.instance_id
//                                 );
//                                 continue;
//                             }
//                         }
//                     }
//                 }
//                 None => {
//                     error!(
//                         "<{:?}>: Sender end unexpectedly closed. Protocol instance will quit.",
//                         &self.instance_id
//                     );
//                     self.terminate().await?;
//                     return Err(ProtocolError::InternalError);
//                 }
//             }
//         }
//     }

//     async fn terminate(&mut self) -> Result<(), ProtocolError> {
//         info!("<{:?}>: Instance finished.", &self.instance_id);
//         self.chan_in.close();
//         // while let Some(share) = self.chan_in.recv().await {
//         //     println!(">> PROT: instance_id: {:?} unused share with share_id: {:?}", &self.instance_id, DecryptionShare::deserialize(&share).get_id());
//         // }
//         Ok(())
//     }
// }
