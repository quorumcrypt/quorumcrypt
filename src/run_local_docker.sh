#!/bin/bash

function start_node {
	my_id=$1

	echo "Starting container for node ${my_id}"

	echo "PWD: $(pwd)"

	docker run \
		--name "server_${my_id}" --detach \
		-e RUST_LOG=debug \
		-v $(pwd)/conf:/mnt/conf \
		-v $(pwd)/mounts:/mnt/out \
		-p 127.0.0.1:5000${my_id}:50000 -p 127.0.0.1:5100${my_id}:51000 \
		--network quorumcrypt \
		--ip "172.21.0.1${my_id}" \
		--entrypoint=server \
		--cpus=1 \
		gitlab.censored.tld:5001/crypto/2021.quorumcrypt:latest \
		--config-file /mnt/conf/server_${my_id}.json \
		--key-file /mnt/conf/keys/node${my_id}.keystore
}

mkdir -p mounts/{0,1,2,3}

start_node 0
start_node 1
start_node 2
start_node 3

echo "Stack is ready. Press enter to shut it down"
read

docker rm -f server_0
docker rm -f server_1
docker rm -f server_2
docker rm -f server_3
