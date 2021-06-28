#!/bin/bash
# There are 3 PeerIds passed to nodes, so I changed it to be 3 at most
if [ -z "$1" ] || (("$1" < 2 || "$1" > 3))
then
    echo "The committee size is missing, usage:

    ./run_nodes.sh SIZE [Additional Arguments to ./target/debug/aleph-node]

where 2 <= SIZE <= 3"
    exit
fi

killall -9 aleph-node

set -e

clear

n_members="$1"
echo "$n_members" > /tmp/n_members
shift

cargo build -p aleph-node

authorities=(Damian Tomasz Zbyszko Hansu Adam Matt Antoni Michal)
authorities=("${authorities[@]::$n_members}")

./target/debug/aleph-node dev-keys  --base-path /tmp --chain dev --key-types aura alp0

# reserved-nodes adds nodes to reserved nodes list
# reserved-only prevents auto detection and nodes not on reserved list from connecting
for i in ${!authorities[@]}; do
  auth=${authorities[$i]}
  ./target/debug/aleph-node purge-chain --base-path /tmp/"$auth" --chain dev -y
  ./target/debug/aleph-node \
    --validator \
    --chain dev \
    --base-path /tmp/$auth \
    --name $auth \
    --rpc-port $(expr 9933 + $i) \
    --ws-port $(expr 9944 + $i) \
    --reserved-nodes /ip4/0.0.0.0/tcp/30334/p2p/12D3KooWNy9S7J3EKsLvjQmFaY8Q99oo5uFaNAM7fuFfUYk6mFn6 /ip4/0.0.0.0/tcp/30335/p2p/12D3KooWLRZYz3hSCcZBo2SvNWZNvAb2gc1kF5E2f343GxFwwpyS /ip4/0.0.0.0/tcp/30336/p2p/12D3KooWD2D2hzEWuH84RSGKuZP4MnNtSRdSNdq16W5LVpkoGNT3 \
    --reserved-only \
    --port $(expr 30334 + $i) \
    --execution Native \
    "$@" \
    2> $auth-$i.log  & \
done

# This node shouldn't connect with the rest
./target/debug/aleph-node purge-chain --base-path /tmp/Michal --chain dev -y
./target/debug/aleph-node \
  --validator \
  --chain dev \
  --base-path /tmp/Michal \
  --name Michal \
  --rpc-port $(expr 9933 + 4) \
  --ws-port $(expr 9944 + 4) \
  --port $(expr 30334 + 4) \
  --execution Native \
  "$@" \
  2> Michal-4.log  & \
