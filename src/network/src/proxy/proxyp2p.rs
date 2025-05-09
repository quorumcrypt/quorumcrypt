use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;

use log::{info, error};
use quorum_proto::proxy_api::proxy_api_server::{ProxyApi, ProxyApiServer};
use quorum_proto::proxy_api::{AtomicBroadcastRequest, AtomicBroadcastResponse, ForwardShareResponse};
// Tokio
use tokio::sync::mpsc::{Receiver, Sender};

use quorum_proto::proxy_api::{
    proxy_api_client::ProxyApiClient, ForwardShareRequest,
};
use tonic::transport::Server;

use crate::interface::Gossip;
use crate::types::config::NetworkConfig;
// Quorumcrypt
use crate::types::message::NetMessage;

use tonic::{async_trait, Request, Response, Status};

pub struct P2PProxy {
    pub config: NetworkConfig,
    pub id: u32,
    sender: Sender<Vec<u8>>,
    receiver: Receiver<Vec<u8>>
}

#[async_trait]
impl Gossip for P2PProxy {

    type T = NetMessage;

    async fn init(&mut self) -> Result<(), String> {

        let local_node = self.config.local_peer.clone();

        // Bind to all interfaces which are available
        let host_ip = <Ipv4Addr>::from_str("0.0.0.0").unwrap();
        let port: u16 = local_node.port;
        let address = SocketAddr::new(IpAddr::V4(host_ip), port);

        info!("[P2PProxyServer]: Request handler is starting. Listening for RPC on address: {address}");
        let service = P2PProxyService{
            sender: self.sender.clone()
        };
        tokio::spawn(async move {
            info!("[P2PProxyServer] Server is starting");
            Server::builder()
                .add_service(ProxyApiServer::new(service))
                // .serve(format!("[{rpc_listen_address}]:{rpc_listen_port}").parse().unwrap())
                .serve(address)
                .await
                .expect("Error starting the gRPC Server!");
        });
        
        Ok(())
    }

    fn broadcast(&mut self, message: NetMessage) -> Result<(), std::string::String> {
        info!("Receiving message from outgoing_channel");
        //here goes the target_platform ip

        let proxy_node = self.config.proxy.as_ref().unwrap().clone();
        let port: u16 = proxy_node.port;
        let ip = proxy_node.ip;

        let address: String = format!("http://{}:{}",ip, port)
        .parse()
        .expect(&format!(
            ">> Fatal error: Could not format address for ip:{}, and port {}.",
            ip,
            port
        ));

        info!("Connecting to remote address: {}", address);
        tokio::spawn(async move {
            match ProxyApiClient::connect(address).await {
                Ok(mut client) => {
                    println!("Id of the msg {}", message.get_instace_id().clone());
                    let request = ForwardShareRequest {
                        data: Vec::from(message),
                    };

                    tokio::spawn(async move { client.forward_share(request).await });
                    Ok(())
                },
                Err(e) => {
                    error!("Error in opening the connection!: {}", e);
                    return Err("Error in opening the connection!".to_string());
                },
            }
        });
        Ok(())
    }

    async fn deliver(&mut self) -> Option<NetMessage> {
        tokio::select! {
            Some(message) = self.receiver.recv() => {
                let msg = NetMessage::from(message);
                info!("Deliver message to the protocol layer");
                return Some(msg)
            }
        }
    }
}


struct P2PProxyService{
    sender: Sender<Vec<u8>>
}

#[tonic::async_trait]
impl ProxyApi for P2PProxyService {
    async fn forward_share(
        &self,
        request: Request<ForwardShareRequest>,
    ) -> Result<Response<ForwardShareResponse>, Status> {
        
        //deliver to the higher layer
        let binding = request.into_inner();
        let msg = binding.data;

        info!("Share received from proxy");
        let _ = self.sender.send(msg).await;

        Ok(Response::new(ForwardShareResponse{}))
    }

    async fn atomic_broadcast(
        &self,
        request: Request<AtomicBroadcastRequest>,
    ) -> Result<Response<AtomicBroadcastResponse>, Status> {
        
        Ok(Response::new(AtomicBroadcastResponse {}))
    }

}

impl P2PProxy {

    pub fn new(config: NetworkConfig, id: u32) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        return P2PProxy { config: config, id: id , sender: sender, receiver: receiver}
    }
}

//TODO: finish the implementation deciding if it is the blockchain part that should push when something is decided (maybe here consider the finality)
pub struct ProxyTOBStub {
    pub config: NetworkConfig,
    pub id: u32,
}

// #[async_trait]
// impl TOB for ProxyTOBStub {

//     type T = NetMessage;

//     fn broadcast(&mut self, message: Self::T){
//         info!("Receiving message from outgoing_channel");
//         //here goes the target_platform ip
//         let mut address = self.config.proxy_addr.clone().to_owned();
//         address.push(':');
//         address.push_str(&self.config.proxy_port.to_string());
//         info!("Connecting to remote address: {}", address);
//         tokio::spawn(async move {
//             match BlockchainStubClient::connect(address).await {
//                 Ok(mut client) => {
//                     println!("Id of the msg {}", message.get_instace_id().clone());
//                     let request = AtomicBroadcastRequest {
//                         id: message.get_instace_id().to_string(),
//                         data: Vec::from(message),
//                     };

//                     tokio::spawn(async move { client.atomic_broadcast(request).await });
//                 }
//                 Err(e) => println!("Error in opening the connection!: {}", e),
//             }
//         });
//     }
//     async fn deliver(&self) -> Option<Self::T>{
//         todo!() //poll the blockchain state
//     }
// }


impl ProxyTOBStub {

    pub fn new(config: NetworkConfig, id: u32) -> Self {
        return ProxyTOBStub{
            config,
            id,
        }
    }

    //setup of service listening for incoming messages
    fn init(){
        todo!()
    }
}

