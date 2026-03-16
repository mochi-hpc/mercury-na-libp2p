#!/bin/bash
#
# DCUtR hole-punch verification driver.
#
# Proves that DCUtR establishes a direct connection by killing the relay
# after lookup and verifying that messaging still works.
#
# Usage: test_dcutr_driver.sh <relay_cmd...> -- <server_cmd...> -- <client_cmd...>
#
set -e

# Parse arguments at "--" separators into 3 groups
RELAY_ARGS=()
SERVER_ARGS=()
CLIENT_ARGS=()
group=0
for arg in "$@"; do
    if [ "$arg" = "--" ]; then
        group=$((group + 1))
        continue
    fi
    case $group in
        0) RELAY_ARGS+=("$arg") ;;
        1) SERVER_ARGS+=("$arg") ;;
        2) CLIENT_ARGS+=("$arg") ;;
    esac
done

if [ ${#RELAY_ARGS[@]} -eq 0 ] || [ ${#SERVER_ARGS[@]} -eq 0 ] || [ ${#CLIENT_ARGS[@]} -eq 0 ]; then
    echo "Usage: $0 <relay_cmd...> -- <server_cmd...> -- <client_cmd...>"
    exit 1
fi

# Clean up on exit
RELAY_PID=""
SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    if [ -n "$RELAY_PID" ] && kill -0 "$RELAY_PID" 2>/dev/null; then
        kill "$RELAY_PID" 2>/dev/null || true
        wait "$RELAY_PID" 2>/dev/null || true
    fi
    rm -f relay_addr.txt na_test_addr.txt dcutr_lookup_done relay_killed
}
trap cleanup EXIT

# Clean up stale files
rm -f relay_addr.txt na_test_addr.txt dcutr_lookup_done relay_killed

# 1. Start relay server
echo "=== Starting relay server ==="
"${RELAY_ARGS[@]}" &
RELAY_PID=$!

# Wait for relay address file (up to 15s)
for i in $(seq 1 150); do
    if [ -f relay_addr.txt ] && [ -s relay_addr.txt ]; then
        break
    fi
    sleep 0.1
done

if [ ! -f relay_addr.txt ] || [ ! -s relay_addr.txt ]; then
    echo "Timed out waiting for relay address file"
    exit 1
fi

RELAY_ADDR=$(cat relay_addr.txt)
echo "=== Relay address: $RELAY_ADDR ==="

# 2. Start NA server with relay address in environment
export MERCURY_RELAY_ADDR="$RELAY_ADDR"
echo "=== Starting NA server ==="
"${SERVER_ARGS[@]}" &
SERVER_PID=$!

# Wait for server to initialize and make relay reservation
sleep 3

if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "NA server exited early"
    wait "$SERVER_PID" 2>/dev/null
    exit 1
fi

# 3. Run NA client in background (it will block on lookup for DCUtR)
echo "=== Starting NA client ==="
"${CLIENT_ARGS[@]}" &
CLIENT_PID=$!

# 4. Wait for the client to signal that lookup (DCUtR) is done
echo "=== Waiting for client to complete DCUtR lookup ==="
for i in $(seq 1 300); do  # up to 30s
    if [ -f dcutr_lookup_done ]; then
        break
    fi
    # Check client hasn't died
    if ! kill -0 "$CLIENT_PID" 2>/dev/null; then
        echo "Client exited before signaling lookup done"
        wait "$CLIENT_PID" 2>/dev/null
        exit 1
    fi
    sleep 0.1
done

if [ ! -f dcutr_lookup_done ]; then
    echo "Timed out waiting for client DCUtR lookup"
    kill "$CLIENT_PID" 2>/dev/null || true
    exit 1
fi

echo "=== Client lookup done — killing relay server ==="

# 5. Kill the relay server
kill "$RELAY_PID" 2>/dev/null || true
wait "$RELAY_PID" 2>/dev/null || true
RELAY_PID=""

# Small delay for TCP close to propagate
sleep 1

# Signal the client that the relay is dead
echo "done" > relay_killed
echo "=== Relay killed — client will now exchange messages over direct connection ==="

# 6. Wait for client to finish
CLIENT_RC=0
wait "$CLIENT_PID" || CLIENT_RC=$?

# 7. Wait for server to finish
SERVER_RC=0
wait "$SERVER_PID" || SERVER_RC=$?
SERVER_PID=""

if [ $SERVER_RC -ne 0 ] || [ $CLIENT_RC -ne 0 ]; then
    echo "FAILED (server=$SERVER_RC client=$CLIENT_RC)"
    exit 1
fi

echo "=== DCUTR TEST PASSED — messaging works after relay killed ==="
