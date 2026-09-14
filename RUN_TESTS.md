# System Execution & Verification Guide

This document contains step-by-step instructions to compile, run, and verify the P2P Mesh Storage Network, both locally (on a single machine using multiple processes) and across multiple physical devices on the same Wi-Fi network.

---

## 1. Running Unit Tests

To run the `/core` logic tests (chunking, Reed-Solomon encoding, AES-GCM encryption, and Merkle tree generation):

```powershell
# In PowerShell (using custom target directory to avoid Windows OneDrive file locks):
$env:CARGO_TARGET_DIR="$env:USERPROFILE\.gemini\antigravity-ide\scratch\cargo-target"
cargo test
```

This will run all 12 tests in `/core`, including the 20MB file round-trip reconstruction test.

---

## 2. Testing Locally (Multi-Process on One Machine)

Testing with multiple local processes is the easiest way to verify P2P coordination, Kademlia routing, and shard distribution.

### Step A: Start Node 1 (Anchor Node)
Start the first node. By default, it uses P2P port `4001` and HTTP API port `3000`.

```powershell
$env:CARGO_TARGET_DIR="$env:USERPROFILE\.gemini\antigravity-ide\scratch\cargo-target"
cargo run -p mesh-node -- --port 4001 --api-port 3000 --quota 1.5
```

Keep this terminal open. Look for the console logs:
* `Local Peer ID: <PEER_ID_1>` (e.g., `12D3KooWN1Xy...`)
* `Starting HTTP API server on 0.0.0.0:3000`
* `Swarm listening on address: /ip4/127.0.0.1/tcp/4001` (and other local IP addresses)

### Step B: Start Node 2 (Peer Node)
Start the second node on different ports (P2P port `4002` and HTTP API port `3001`):

```powershell
$env:CARGO_TARGET_DIR="$env:USERPROFILE\.gemini\antigravity-ide\scratch\cargo-target"
cargo run -p mesh-node -- --port 4002 --api-port 3001 --quota 1.5
```

Keep this terminal open. Look for:
* `Local Peer ID: <PEER_ID_2>` (e.g., `12D3KooWR45c...`)
* `Starting HTTP API server on 0.0.0.0:3001`
* `Swarm listening on address: /ip4/127.0.0.1/tcp/4002`

### Step C: Pair Node 1 and Node 2
Because of the **Noise transport authentication**, nodes will reject connection handshakes from untrusted Peer IDs. We need to trigger the **pairing exchange** between them.

Use PowerShell or `curl` to pair Node 1 to Node 2:

* **Using PowerShell (PowerShell 6+ / Core)**:
  ```powershell
  $body = @{ multiaddr = "/ip4/127.0.0.1/tcp/4002/p2p/<PEER_ID_2>" } | ConvertTo-Json
  Invoke-RestMethod -Uri "http://localhost:3000/pair" -Method Post -ContentType "application/json" -Body $body
  ```

* **Using curl**:
  ```bash
  curl -X POST -H "Content-Type: application/json" -d '{"multiaddr": "/ip4/127.0.0.1/tcp/4002/p2p/<PEER_ID_2>"}' http://localhost:3000/pair
  ```

*(Replace `<PEER_ID_2>` with the actual Peer ID printed in Node 2's terminal.)*

**What happens under the hood**:
Node 1 dials Node 2, completes the Noise handshake, adds Node 2 to its trusted list, sends a pairing command, and Node 2 adds Node 1 to its trusted list in return. Both nodes save their peer IDs into `./data_4001/trusted_peers.json` and `./data_4002/trusted_peers.json` respectively.

### Step D: Verify Pairing Status
Check if they see each other in their `/peers` status lists:

```bash
# Get status of Node 1
curl http://localhost:3000/status

# Get status of Node 2
curl http://localhost:3001/status
```
You should see each node listed in the other's `"peers"` array.

### Step E: Test File Upload (Sharding & Distribution)
Upload a test file to Node 1. We'll chunk and encode it with `k=2, m=1` (minimum 3 peers are needed for $k+m=3$; for 2 peers, use `k=1, m=1`):

1. Create a dummy file:
   ```powershell
   "Hello world, this is a test file for the decentralized mesh storage network!" > test.txt
   ```
2. Upload it to Node 1 using `k=1` (1 data shard) and `m=1` (1 parity shard):
   ```powershell
   # PowerShell Form Upload:
   Invoke-RestMethod -Uri "http://localhost:3000/upload" -Method Post -Form @{
       file_id = "my-doc-1"
       passphrase = "password123"
       salt = "some-salt-key"
       k = "1"
       m = "1"
       file = Get-Item "test.txt"
   }
   ```
   *Alternative curl Command*:
   ```bash
   curl -F "file_id=my-doc-1" -F "passphrase=password123" -F "salt=some-salt-key" -F "k=1" -F "m=1" -F "file=@test.txt" http://localhost:3000/upload
   ```

3. **Check Shard Placement**:
   You will receive a JSON response showing the manifest details and shard hashes.
   Check `/shards` on Node 1:
   ```bash
   curl http://localhost:3000/shards
   ```
   Check `/shards` on Node 2:
   ```bash
   curl http://localhost:3001/shards
   ```
   One shard of the chunk will be stored in `./data_4001/shards/` and the other in `./data_4002/shards/`.

### Step F: Test File Download & Reconstruction
Now request the download of the file through Node 2 (which has to fetch the other shard from Node 1 to reconstruct it):

* **Using PowerShell**:
  ```powershell
  Invoke-RestMethod -Uri "http://localhost:3001/download/my-doc-1?passphrase=password123&salt=some-salt-key&k=1&m=1"
  ```
* **Using curl**:
  ```bash
  curl "http://localhost:3001/download/my-doc-1?passphrase=password123&salt=some-salt-key&k=1&m=1"
  ```

The terminal should print: `Hello world, this is a test file for the decentralized mesh storage network!`

---

## 3. Testing Across Different Devices (e.g. PC + Phone/Laptop)

To test the system on different physical devices (e.g., your Windows PC and another laptop or a virtual machine on the same LAN):

### Step A: Configure Firewall & Netsh
The nodes must bind to `0.0.0.0` (which is default) and allow incoming TCP traffic. Ensure your OS firewall allows TCP ports `4001` (p2p) and `3000` (HTTP) or whatever port you choose.
*On Windows, you can add an inbound rule manually or let the app prompt for access.*

### Step B: Launch Node on Device A (Anchor Node - PC)
Find Device A's local LAN IP address (e.g. `192.168.1.10` via `ipconfig`):
```powershell
cargo run -p mesh-node -- --port 4001 --api-port 3000
```
Note the Peer ID: e.g. `12D3KooW-PC-PEER-ID`.

### Step C: Launch Node on Device B (Laptop/Device B)
Find Device B's local LAN IP address (e.g. `192.168.1.15`). Run:
```bash
cargo run -p mesh-node -- --port 4001 --api-port 3000
```
Note the Peer ID: e.g. `12D3KooW-LAPTOP-PEER-ID`.

### Step D: Pair Device B with Device A
Send an API request to Device B's API (`http://192.168.1.15:3000`) instructing it to pair with Device A:

* **Using curl on Device B**:
  ```bash
  curl -X POST -H "Content-Type: application/json" -d '{"multiaddr": "/ip4/192.168.1.10/tcp/4001/p2p/12D3KooW-PC-PEER-ID"}' http://localhost:3000/pair
  ```

Once paired:
* Device B will trust Device A, and Device A will trust Device B.
* The respective Peer IDs are added to `trusted_peers.json` on both devices.
* mDNS will automatically discover and sync Kademlia routing entries when the devices are on the same Wi-Fi.

### Step E: Upload & Retrieve Across Devices
1. Upload a file on Device A:
   ```bash
   curl -F "file_id=lan-doc" -F "passphrase=securepass" -F "salt=mysalt" -F "k=1" -F "m=1" -F "file=@somefile.zip" http://localhost:3000/upload
   ```
2. Retrieve the file on Device B:
   ```bash
   curl -o downloaded.zip "http://localhost:3000/download/lan-doc?passphrase=securepass&salt=mysalt&k=1&m=1"
   ```
   Even though you uploaded on Device A, Device B can fetch the manifest, download the shards over the P2P connection, verify integrity, reconstruct, decrypt, and save the zip file.

---

## 4. Verifying Security & Anti-Flood Banning

To test the security system and automatic IP banning:

1. **Attempt Connections with an Untrusted Node**:
   If a node that has not completed the `/pair` exchange attempts to connect to a running node, it is disconnected immediately on connection establishment.
2. **Brute Force Detection**:
   If an untrusted node makes **5 failed connection attempts within 60 seconds**:
   * The victim node bans the attacker's IP for 30 minutes in-memory.
   * If running with Administrator privileges on Windows, the node automatically executes `netsh advfirewall firewall add rule...` to block the attacker's IP at the OS layer.
