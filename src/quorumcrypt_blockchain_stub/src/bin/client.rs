use quorum_proto::proxy_api::{proxy_api_client::ProxyApiClient, ForwardShareRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello there! I'm the client!");

    let mut client = ProxyApiClient::connect("http://localhost:30000").await?;
    let mut client1 = ProxyApiClient::connect("http://localhost:30001").await?;
    let mut client2 = ProxyApiClient::connect("http://localhost:30002").await?;
    let mut client3 = ProxyApiClient::connect("http://localhost:30003").await?;

    let request = ForwardShareRequest{
        data: "Hello World".to_owned().into_bytes(),
    };

    let request1 = ForwardShareRequest{
        data: "Hello World 1".to_owned().into_bytes(),
    };

    let request2 = ForwardShareRequest{
        data: "Hello World 2".to_owned().into_bytes(),
    };

    let request3 = ForwardShareRequest{
        data: "Hello World 3".to_owned().into_bytes(),
    };



    let response = client.forward_share(request).await.expect("Client RPC request failed");
    let response1 = client1.forward_share(request1).await.expect("Client RPC request failed");
    let response2 = client2.forward_share(request2).await.expect("Client RPC request failed");
    let response3 = client3.forward_share(request3).await.expect("Client RPC request failed");

    println!("RESPONSE={:?}", response);
    Ok(())
}
