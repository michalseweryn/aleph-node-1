#!/bin/bash

if [ -z "$2" ] || (("$1" < 2 || "$2" < 0 || $(expr "$1" + "$2") > 10))
then
    echo "Usage:
    ./run_nodes.sh n_validators n_non_validators [Additional Arguments to ./target/debug/aleph-node]
where 2 <= n_validators <= n_validators + n_non_validators <= 10"
    exit
fi

killall -9 aleph-node

set -e

clear

n_validators="$1"
n_non_validators="$2"
shift
shift

# cargo build --release -p aleph-node

account_ids=(
    "5D34dL5prEUaGNQtPPZ3yN5Y6BnkfXunKXXz6fo7ZJbLwRRH"
    "5GBNeWRhZc2jXu7D55rBimKYDk8PGk8itRYFTPfC8RJLKG5o" \
    "5Dfis6XL8J2P6JHUnUtArnFWndn62SydeP8ee8sG2ky9nfm9" \
    "5F4H97f7nQovyrbiq4ZetaaviNwThSVcFobcA5aGab6167dK" \
    "5DiDShBWa1fQx6gLzpf3SFBhMinCoyvHM1BWjPNsmXS8hkrW" \
    "5EFb84yH9tpcFuiKUcsmdoF7xeeY3ajG1ZLQimxQoFt9HMKR" \
    "5DZLHESsfGrJ5YzT3HuRPXsSNb589xQ4Unubh1mYLodzKdVY" \
    "5GHJzqvG6tXnngCpG7B12qjUvbo5e4e9z8Xjidk3CQZHxTPZ" \
    "5CUnSsgAyLND3bxxnfNhgWXSe9Wn676JzLpGLgyJv858qhoX" \
    "5CVKn7HAZW1Ky4r7Vkgsr7VEW88C2sHgUNDiwHY9Ct2hjU8q")
validator_ids=("${account_ids[@]::$n_validators}")
# space separated ids
validator_ids_string="${validator_ids[*]}"
# comma separated ids
validator_ids_string="${validator_ids_string//${IFS:0:1}/,}"


./target/release/aleph-node bootstrap-chain --base-path docker/data --chain-id dev --account-ids "$validator_ids_string" > docker/data/chainspec.json



for i in $(seq "$n_validators" "$(( n_validators + n_non_validators - 1 ))"); do
  echo "Bootstrapping node $i"
  account_id=${account_ids[$i]}
  ./target/release/aleph-node bootstrap-node --base-path docker/data --chain-id dev --account-id "$account_id"
done

for i in $(seq 0 "$(( n_validators + n_non_validators - 1 ))"); do
  auth="node-$i"
  account_id=${account_ids[$i]}
  ./target/release/aleph-node purge-chain --base-path "docker/data/$account_id" --chain docker/data/chainspec.json -y
  ./target/release/aleph-node \
    --validator \
    --chain docker/data/chainspec.json \
    --base-path "docker/data/$account_id" \
    --name "$auth" \
    --rpc-port "$((9933 + i))" \
    --ws-port "$((9944 + i))" \
    --port "$((30334 + i))" \
    --execution Native \
    -lafa=debug \
    "$@" \
    2> "$auth.log" > /dev/null & \
done
