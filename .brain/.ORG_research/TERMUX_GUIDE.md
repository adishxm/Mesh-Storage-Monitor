# Running a Mesh Storage Node on Android (via Termux)

This guide walks you through compiling and running the P2P Mesh Storage Node directly on an Android device using **Termux**. By running a native node on your phone (Option B), your phone becomes an active participant in the storage network, storing real encrypted file pieces.

Even if you turn your PC completely off, your phone can still reconstruct and serve files it holds shards of.

---

## Step 1: Install & Set Up Termux

> [!WARNING]
> Do **NOT** install Termux from the Google Play Store (it is outdated and compile tools will fail). Always download it from **F-Droid** or GitHub.

1. Download the Termux APK from [F-Droid](https://f-droid.org/en/packages/termux.termux/) or directly from [Termux GitHub Releases](https://github.com/termux/termux-app/releases).
2. Open Termux on your phone and run the following command to update packages:
   ```bash
   pkg update && pkg upgrade -y
   ```
3. (Optional) Enable storage permissions so Termux can access your phone's main files:
   ```bash
   termux-setup-storage
   ```

---

## Step 2: Install Rust and Compiler Tools

Rust and build tools are required to compile the binaries on Android:
```bash
pkg install rust clang build-essential git -y
```

Verify your installation works by checking the versions:
```bash
rustc --version
cargo --version
```

---

## Step 3: Transfer the Code to Your Phone

You can get the project folder onto your phone in a few ways:

### Option A: Via Git (Recommended)
If your repository is pushed to GitHub or a local server:
```bash
git clone <your-repo-url> p2p-network
cd p2p-network
```

### Option B: Local Wi-Fi Transfer (SSH)
1. In Termux, install and start SSH server:
   ```bash
   pkg install openssh -y
   sshd
   whoami     # Note your username (e.g., u0_a123)
   passwd     # Set a password for ssh login
   ```
2. On your Windows PC, transfer the project directory using SFTP or `scp`:
   ```powershell
   scp -r -P 8022 C:\Users\adity\OneDrive\Desktop\p2p-network u0_a123@<PHONE_IP>:~/p2p-network
   ```
   *(Find `<PHONE_IP>` in Termux by running `ifconfig` or `ip addr show wlan0`)*

---

## Step 4: Build the Node

Once in the project directory in Termux, compile the node in release mode:
```bash
cargo build -p mesh-node --release
```
*(This may take a few minutes on a mobile device as it compiles dependencies. Rust's output directory on Termux defaults to the standard target path since Windows OneDrive file locks do not apply).*

---

## Step 5: Find Your Phone's LAN IP Address

Make sure both your PC and your phone are connected to the **same Wi-Fi network**.
In Termux, check your local IP:
```bash
ip addr show wlan0 | grep "inet "
```
Look for an IP like `192.168.1.15` or similar.

---

## Step 6: Launch Node on Android

Run the built node. Let's use P2P port `4003` and API port `3002`:
```bash
./target/release/mesh-node --port 4003 --api-port 3002
```
Look for:
* `Local Peer ID: <PHONE_PEER_ID>`
* `Swarm listening on address: /ip4/192.168.1.15/tcp/4003`

---

## Step 7: Pair Your PC and Phone

You need to establish mutual trust between the PC and the phone before they will route shards.

### From Your PC:
Open a terminal on your PC and run:
```powershell
# In PowerShell:
$body = @{ multiaddr = "/ip4/<PHONE_IP>/tcp/4003/p2p/<PHONE_PEER_ID>" } | ConvertTo-Json
Invoke-RestMethod -Uri "http://localhost:3000/pair" -Method Post -ContentType "application/json" -Body $body
```
*(Or use `curl` equivalent replacing with the phone's actual IP and Peer ID).*

### Verify Trust:
On your PC:
```bash
curl http://localhost:3000/status
```
You should see your Phone's Peer ID in the `peers` and `trusted_peers` lists.

---

## Step 8: Upload and Test Offline Recovery

1. **Upload a file** from your PC to Node 1 (using `k=1, m=1` so 1 data shard is stored on the PC and 1 backup shard is sent to the phone):
   ```bash
   curl -F "file_id=test-doc-phone" -F "passphrase=supersecret" -F "salt=mysalt" -F "k=1" -F "m=1" -F "file=@test.txt" http://localhost:3000/upload
   ```
2. **Verify storage**: Check `/shards` on your phone's API port (`http://<PHONE_IP>:3002/shards`). You will see a shard hash file listed!
3. **Go Offline**: Turn off the node running on your PC (press `Ctrl+C` in Node 1's terminal, or shut the PC down).
4. **Retrieve File from Phone**:
   Run the download request directly against the phone's API:
   ```bash
   curl "http://<PHONE_IP>:3002/download/test-doc-phone?passphrase=supersecret&salt=mysalt&k=1&m=1"
   ```
   Even though the PC node is down, the phone has enough shards (`k=1`) to reconstruct the decrypted plaintext file and serve it back!
