import net from 'net';
import dgram from 'dgram';
import crypto from 'crypto';
import { exec } from 'child_process';
import { getPublicKeyFromPeerId, verifySignature, signData } from './core.js';

// Message buffer framing: [4-byte totalLen] + [4-byte headerLen] + [JSON Header] + [Optional Binary Payload]
class MessageBuffer {
  constructor(callback) {
    this.buffer = Buffer.alloc(0);
    this.callback = callback;
  }

  push(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 8) {
      const totalLen = this.buffer.readUInt32BE(0);
      const headerLen = this.buffer.readUInt32BE(4);
      if (this.buffer.length >= 8 + totalLen) {
        const headerBytes = this.buffer.subarray(8, 8 + headerLen);
        const payloadBytes = this.buffer.subarray(8 + headerLen, 8 + totalLen);
        this.buffer = this.buffer.subarray(8 + totalLen);
        try {
          const header = JSON.parse(headerBytes.toString('utf8'));
          this.callback(header, payloadBytes.length > 0 ? payloadBytes : null);
        } catch (e) {
          console.error('Failed to parse P2P message frame:', e.message);
        }
      } else {
        break;
      }
    }
  }
}

// Writes a framed message to a TCP socket
export function writeMessage(socket, header, payload = null) {
  if (socket.destroyed) return;
  const headerBytes = Buffer.from(JSON.stringify(header), 'utf8');
  const payloadBytes = payload ? (Buffer.isBuffer(payload) ? payload : Buffer.from(payload)) : Buffer.alloc(0);
  
  const totalLen = headerBytes.length + payloadBytes.length;
  const lenBuf = Buffer.alloc(8);
  lenBuf.writeUInt32BE(totalLen, 0);
  lenBuf.writeUInt32BE(headerBytes.length, 4);
  
  socket.write(Buffer.concat([lenBuf, headerBytes, payloadBytes]));
}

export class NetworkService {
  constructor(keypair, state) {
    this.keypair = keypair; // { privateKey, publicKey, rawPub, peerId }
    this.state = state; // NodeState instance
    
    this.server = null;
    this.sockets = new Map(); // PeerId string -> TCP socket
    this.pendingRequests = new Map(); // reqId -> { resolve, reject, timer }
    this.unlistedAttempts = new Map(); // IP -> { count, timestamp }
    this.blockedIps = new Map(); // IP -> block expiry timestamp
    this.seenGossip = new Set(); // Set of gossip message hashes
    this.udpSocket = null;
    this.requestIdCounter = 0;
  }

  start(p2pPort) {
    // 1. Start TCP server
    this.server = net.createServer((socket) => {
      this.handleIncomingConnection(socket);
    });

    this.server.listen(p2pPort, '0.0.0.0', () => {
      console.log(`P2P TCP server listening on port ${p2pPort}`);
      const ip = '0.0.0.0';
      this.state.listenAddresses.add(`/ip4/${ip}/tcp/${p2pPort}/p2p/${this.keypair.peerId}`);
    });

    // 2. Start UDP Multicast discovery
    this.startDiscovery(p2pPort);

    // 3. Start Heartbeat Ping Loop (every 5 seconds)
    setInterval(() => this.pingConnectedPeers(), 5000);

    // 4. Start Integrity Audit Loop (every 30 seconds)
    setInterval(() => this.runIntegrityAudits(), 30000);

    // 5. Cleanup expired IP bans (every 60 seconds)
    setInterval(() => this.cleanupBlockedIps(), 60000);
  }

  // ==========================================
  // OS-LEVEL FIREWALL BANS
  // ==========================================
  blockIpFirewall(ip) {
    console.log(`OS firewall block triggered for IP: ${ip}`);
    
    // Windows block
    if (process.platform === 'win32') {
      const ruleName = `MeshStorage_Block_${ip.replace(/\./g, '_')}`;
      const cmd = `netsh advfirewall firewall add rule name="${ruleName}" dir=in action=block remoteip=${ip}`;
      exec(cmd, (err, stdout, stderr) => {
        if (err) {
          console.warn(`Windows Firewall ban command failed (requires Admin privileges): ${stderr.trim()}`);
        } else {
          console.log(`Successfully added Windows Firewall block rule for IP: ${ip}`);
        }
      });
    } 
    // Linux block
    else if (process.platform === 'linux' || process.platform === 'android') {
      const cmd = `iptables -A INPUT -s ${ip} -j DROP`;
      exec(cmd, (err, stdout, stderr) => {
        if (err) {
          console.warn(`Linux iptables ban command failed (requires root/sudo): ${stderr.trim()}`);
        } else {
          console.log(`Successfully added iptables block rule for IP: ${ip}`);
        }
      });
    } 
    // Others
    else {
      console.log(`OS-level firewall blocking not supported on platform: ${process.platform}. In-memory blocking remains active.`);
    }
  }

  cleanupBlockedIps() {
    const now = Date.now();
    for (const [ip, expiry] of this.blockedIps.entries()) {
      if (now > expiry) {
        this.blockedIps.delete(ip);
        console.log(`IP ban expired for ${ip}`);
        // Optionally remove firewall rule if we tracked it, but keeping it simple for prototype.
      }
    }
  }

  // ==========================================
  // UDP MULTICAST AUTO-DISCOVERY
  // ==========================================
  startDiscovery(p2pPort) {
    this.udpSocket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    
    this.udpSocket.on('message', (msg, rinfo) => {
      try {
        const info = JSON.parse(msg.toString('utf8'));
        if (info.peerId === this.keypair.peerId) return; // Skip ourselves
        
        // If we trust this peer and aren't connected, dial them!
        if (this.state.isTrusted(info.peerId) && !this.sockets.has(info.peerId)) {
          console.log(`UDP multicast discovered trusted peer ${info.peerId} at ${rinfo.address}:${info.p2pPort}. Dialing...`);
          this.connectToPeer(rinfo.address, info.p2pPort);
        }
      } catch (e) {
        // Ignore malformed packets
      }
    });

    this.udpSocket.bind(5566, () => {
      try {
        this.udpSocket.addMembership('224.0.2.15');
        console.log('UDP Multicast bound to 224.0.2.15:5566');
      } catch (err) {
        console.warn(`Failed to join multicast group (likely needs network interface configuration): ${err.message}`);
      }
    });

    // Broadcast our presence every 3 seconds
    setInterval(() => {
      if (this.udpSocket) {
        const msg = Buffer.from(JSON.stringify({
          peerId: this.keypair.peerId,
          p2pPort: p2pPort,
          apiPort: this.state.p2pPort - 1000 // Simple api port heuristic
        }));
        this.udpSocket.send(msg, 0, msg.length, 5566, '224.0.2.15', (err) => {
          if (err) {
            // Silence UDP send errors
          }
        });
      }
    }, 3000);
  }

  // ==========================================
  // TCP CONNECTION HANDLERS & HANDSHAKE
  // ==========================================
  handleIncomingConnection(socket) {
    const ip = socket.remoteAddress;

    // Check if IP is blocked
    if (this.blockedIps.has(ip)) {
      const expiry = this.blockedIps.get(ip);
      if (Date.now() < expiry) {
        console.warn(`Incoming connection from banned IP rejected: ${ip}`);
        socket.destroy();
        return;
      }
    }

    console.log(`Incoming connection established from ${ip}:${socket.remotePort}`);
    this.setupSocketLifecycle(socket, true);
  }

  connectToPeer(ip, port) {
    console.log(`Dialing peer at ${ip}:${port}...`);
    const socket = net.connect({ host: ip, port: port }, () => {
      console.log(`TCP connection outbound established to ${ip}:${port}`);
      this.setupSocketLifecycle(socket, false);
    });

    socket.on('error', (err) => {
      console.warn(`Failed to connect to ${ip}:${port}: ${err.message}`);
    });
  }

  setupSocketLifecycle(socket, isIncoming) {
    const ip = socket.remoteAddress;
    const challenge = crypto.randomBytes(16);
    let authenticated = false;
    let peerId = null;

    // Start handshake protocol
    // Step A. Write our challenge first
    writeMessage(socket, { type: 'challenge', challenge: challenge.toString('hex') });

    const msgBuf = new MessageBuffer(async (header, payload) => {
      // Prioritize Handshake
      if (!authenticated) {
        if (header.type === 'challenge') {
          // Send handshake response: sign the challenge they sent us
          const receivedChallenge = Buffer.from(header.challenge, 'hex');
          const toSign = Buffer.concat([receivedChallenge, Buffer.from(this.keypair.peerId, 'utf8')]);
          const signature = signData(this.keypair.privateKey, toSign);

          writeMessage(socket, {
            type: 'handshake',
            peerId: this.keypair.peerId,
            p2pPort: this.state.p2pPort,
            signature: signature.toString('hex')
          });
        } 
        else if (header.type === 'handshake') {
          peerId = header.peerId;
          
          // Verify access control allowlist
          if (!this.state.isTrusted(peerId)) {
            console.warn(`Untrusted Peer ID connection attempt: ${peerId} from IP: ${ip}`);
            socket.destroy();
            this.trackUntrustedAttempt(ip);
            return;
          }

          // Verify handshake signature
          try {
            const pubKey = getPublicKeyFromPeerId(peerId);
            const verified = verifySignature(
              pubKey,
              Buffer.concat([challenge, Buffer.from(peerId, 'utf8')]),
              Buffer.from(header.signature, 'hex')
            );

            if (!verified) {
              console.warn(`Cryptographic handshake signature verification FAILED for peer: ${peerId}`);
              socket.destroy();
              this.trackUntrustedAttempt(ip);
              return;
            }

            // Success! Handshake complete
            authenticated = true;
            socket.peerId = peerId;
            socket.p2pPort = header.p2pPort;
            this.sockets.set(peerId, socket);
            this.state.connectedPeers.add(peerId);
            console.log(`Mutual Ed25519 authentication successful with trusted PeerID: ${peerId}`);
            
            // Send PairAck back to confirm success
            writeMessage(socket, { type: 'pair_ack', success: true });
          } catch (err) {
            console.error(`Handshake processing error: ${err.message}`);
            socket.destroy();
          }
        }
        return;
      }

      // Handled authenticated messages
      this.handlePeerMessage(socket, header, payload);
    });

    socket.on('data', (chunk) => msgBuf.push(chunk));
    
    socket.on('close', () => {
      if (peerId) {
        this.sockets.delete(peerId);
        this.state.connectedPeers.delete(peerId);
        console.log(`Peer ID disconnected: ${peerId}`);
      }
    });

    socket.on('error', (err) => {
      // Silence expected connection resets
    });
  }

  trackUntrustedAttempt(ip) {
    const now = Date.now();
    if (!this.unlistedAttempts.has(ip)) {
      this.unlistedAttempts.set(ip, { count: 1, timestamp: now });
    } else {
      const attempt = this.unlistedAttempts.get(ip);
      if (now - attempt.timestamp < 60000) {
        attempt.count++;
        console.log(`Untrusted connection attempt count for ${ip}: ${attempt.count}/5`);
        if (attempt.count >= 5) {
          console.warn(`IP ${ip} exceeded 5 untrusted connection attempts in 60s. Banning for 30 minutes!`);
          this.blockedIps.set(ip, now + 30 * 60 * 1000); // 30 min expiry
          this.unlistedAttempts.delete(ip);
          this.blockIpFirewall(ip);
        }
      } else {
        // Reset window
        attempt.count = 1;
        attempt.timestamp = now;
      }
    }
  }

  // ==========================================
  // PEER PROTOCOL MESSAGE ROUTER
  // ==========================================
  async handlePeerMessage(socket, header, payload) {
    const { type, id } = header;

    // A. Heartbeat Ping/Pong
    if (type === 'ping') {
      writeMessage(socket, { type: 'pong' });
      return;
    }
    if (type === 'pong') {
      // heartbeat handled implicitly by TCP connection longevity
      return;
    }

    // B. Gossipsub broadcast
    if (type === 'gossip') {
      const msgHash = crypto.createHash('sha256').update(header.dataId + header.topic).digest('hex');
      if (!this.seenGossip.has(msgHash)) {
        this.seenGossip.add(msgHash);
        
        // Log gossip events
        console.log(`Gossip Message [${header.topic}]:`, header.message);
        
        // Relay gossip to all other connected peers
        for (const [peerId, otherSocket] of this.sockets.entries()) {
          if (peerId !== socket.peerId) {
            writeMessage(otherSocket, header);
          }
        }
      }
      return;
    }

    // C. Requests (Client -> Server)
    if (type === 'request') {
      try {
        const result = await this.processRequest(header.method, header.params, payload);
        writeMessage(socket, { type: 'response', id, success: true, result });
      } catch (err) {
        writeMessage(socket, { type: 'response', id, success: false, error: err.message });
      }
      return;
    }

    // D. Responses (Server -> Client)
    if (type === 'response') {
      if (this.pendingRequests.has(id)) {
        const { resolve, reject, timer } = this.pendingRequests.get(id);
        clearTimeout(timer);
        this.pendingRequests.delete(id);
        
        if (header.success) {
          resolve({ result: header.result, payload });
        } else {
          reject(new Error(header.error));
        }
      }
      return;
    }
  }

  // ==========================================
  // PEER REQUESTS PROCESSOR (Server-side)
  // ==========================================
  async processRequest(method, params, payload) {
    // 1. Shard Store
    if (method === 'store') {
      const { shardHash } = params;
      try {
        this.state.writeShard(shardHash, payload);
        console.log(`Stored shard ${shardHash} successfully`);
        return { success: true };
      } catch (err) {
        throw new Error(`Store failed: ${err.message}`);
      }
    }

    // 2. Shard Retrieve
    if (method === 'retrieve') {
      const { shardHash } = params;
      const data = this.state.readShard(shardHash);
      if (!data) throw new Error(`Shard ${shardHash} not found`);
      
      // We return the response with binary payload
      return { success: true, hasPayload: true, payloadLength: data.length };
    }

    // 3. Shard Audit Challenge
    if (method === 'audit_challenge') {
      const { shardHash, nonceHex } = params;
      const data = this.state.readShard(shardHash);
      if (!data) throw new Error('Shard not found');
      
      const nonce = Buffer.from(nonceHex, 'hex');
      const hashInput = Buffer.concat([data, nonce]);
      const auditHash = crypto.createHash('sha256').update(hashInput).digest('hex');
      
      return { auditHash };
    }

    // 4. Pairing exchange
    if (method === 'pair') {
      const { callerMultiaddr } = params;
      console.log(`Exchange pairing completed with ${callerMultiaddr}`);
      return { success: true };
    }

    throw new Error(`Unsupported method: ${method}`);
  }

  // ==========================================
  // CLIENT SEND REQUEST WRAPPER
  // ==========================================
  sendRequest(peerId, method, params = {}, payload = null, timeoutMs = 10000) {
    return new Promise((resolve, reject) => {
      const socket = this.sockets.get(peerId);
      if (!socket) {
        return reject(new Error(`Peer ${peerId} is not connected`));
      }

      const id = `req-${this.keypair.peerId}-${++this.requestIdCounter}`;
      
      const timer = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request ${method} to peer ${peerId} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pendingRequests.set(id, { resolve, reject, timer });
      
      writeMessage(socket, {
        type: 'request',
        id,
        method,
        params
      }, payload);
    });
  }

  // ==========================================
  // GOSSIP BROADCAST
  // ==========================================
  broadcastGossip(topic, message) {
    const dataId = crypto.randomBytes(8).toString('hex');
    const msg = {
      type: 'gossip',
      topic,
      dataId,
      message
    };
    
    const msgHash = crypto.createHash('sha256').update(dataId + topic).digest('hex');
    this.seenGossip.add(msgHash);

    for (const socket of this.sockets.values()) {
      writeMessage(socket, msg);
    }
  }

  // ==========================================
  // LIVENESS HEARTBEATS
  // ==========================================
  pingConnectedPeers() {
    for (const [peerId, socket] of this.sockets.entries()) {
      try {
        writeMessage(socket, { type: 'ping' });
      } catch (err) {
        console.warn(`Error pinging peer ${peerId}: ${err.message}`);
        socket.destroy();
      }
    }
  }

  // ==========================================
  // PAIR PEER ACTION
  // ==========================================
  async pairPeer(multiaddr) {
    // Format: /ip4/192.168.1.15/tcp/4001/p2p/12D3KooW...
    const parts = multiaddr.split('/');
    const ip = parts[2];
    const port = parseInt(parts[4], 10);
    const peerId = parts[6];

    if (!ip || isNaN(port) || !peerId) {
      throw new Error(`Invalid multiaddr format: ${multiaddr}`);
    }

    // Add to trusted allowlist
    this.state.addTrustedPeer(peerId);

    // Dial P2P socket
    this.connectToPeer(ip, port);

    // Wait for connection and handshake to settle
    let attempts = 0;
    while (!this.sockets.has(peerId) && attempts < 10) {
      await new Promise(r => setTimeout(r, 500));
      attempts++;
    }

    if (!this.sockets.has(peerId)) {
      throw new Error('Timeout connecting and authenticating peer');
    }

    // Send pairing request to perform reciprocal add
    const response = await this.sendRequest(peerId, 'pair', {
      callerMultiaddr: `/ip4/127.0.0.1/tcp/${this.state.p2pPort}/p2p/${this.keypair.peerId}`
    });

    if (response.result.success) {
      console.log(`Reciprocal pairing established with Peer: ${peerId}`);
    } else {
      throw new Error('Reciprocal pairing failed');
    }
  }

  // ==========================================
  // PERIODIC INTEGRITY AUDITS & SELF-HEALING
  // ==========================================
  async runIntegrityAudits() {
    console.log('--- Starting Integrity Audit Loop ---');
    const manifests = this.state.listManifests();
    
    for (const manifest of manifests) {
      const fileId = manifest.file_id;
      const k = manifest.k;
      const m = manifest.m;
      let manifestUpdated = false;

      // Only audit files we hold the private keys for (so we can repair them)
      const fileKey = this.state.readFileKey(fileId);
      if (!fileKey) continue;
      
      for (let chunkIdx = 0; chunkIdx < manifest.chunks.length; chunkIdx++) {
        const chunkManifest = manifest.chunks[chunkIdx];
        
        for (let shardIdx = 0; shardIdx < chunkManifest.shard_hashes.length; shardIdx++) {
          const shardHashBytes = chunkManifest.shard_hashes[shardIdx];
          const shardHashHex = Buffer.from(shardHashBytes).toString('hex');
          const holderPeerId = chunkManifest.shard_holders[shardIdx];
          
          if (!holderPeerId) continue; // Shard not placed yet or empty

          let auditSuccess = false;
          
          // Case A: Local Shard Holder
          if (holderPeerId === this.keypair.peerId) {
            if (this.state.hasShard(shardHashHex)) {
              // verify local hash matches
              const localBytes = this.state.readShard(shardHashHex);
              const actualHash = crypto.createHash('sha256').update(localBytes).digest('hex');
              auditSuccess = (actualHash === shardHashHex);
            }
          } 
          // Case B: Remote Shard Holder
          else if (this.sockets.has(holderPeerId)) {
            try {
              // Get pre-computed challenges
              const challengeList = chunkManifest.audits ? chunkManifest.audits[shardIdx] : [];
              if (challengeList && challengeList.length > 0) {
                // Cycle through challenges randomly to prevent predictability
                const challenge = challengeList[Math.floor(Math.random() * challengeList.length)];
                const response = await this.sendRequest(holderPeerId, 'audit_challenge', {
                  shardHash: shardHashHex,
                  nonceHex: challenge.nonce
                }, null, 5000); // 5s timeout
                
                if (response.result && response.result.auditHash === challenge.response) {
                  auditSuccess = true;
                }
              } else {
                // Fallback: If no pre-computed challenge exists, check if peer simply has the shard
                const response = await this.sendRequest(holderPeerId, 'retrieve', {
                  shardHash: shardHashHex
                }, null, 5000);
                if (response.result && response.result.success) {
                  auditSuccess = true;
                }
              }
            } catch (err) {
              console.warn(`Audit check failed/timeout for shard ${shardHashHex.substring(0, 8)} on peer ${holderPeerId.substring(0, 8)}`);
            }
          }

          if (!auditSuccess) {
            console.warn(`⚠️ AUDIT FAILURE: Shard ${shardIdx} of chunk ${chunkIdx} on node ${holderPeerId} is DEGRADED/OFFLINE. Triggering repair...`);
            
            this.broadcastGossip('mesh-events', {
              type: 'audit_failed',
              file_id: fileId,
              chunk_idx: chunkIdx,
              shard_idx: shardIdx,
              failed_node: holderPeerId
            });

            // Perform repair
            try {
              const repaired = await this.repairShard(manifest, chunkIdx, shardIdx, fileKey);
              if (repaired) {
                manifestUpdated = true;
              }
            } catch (repairErr) {
              console.error(`Repair failed for file ${fileId} chunk ${chunkIdx} shard ${shardIdx}: ${repairErr.message}`);
            }
          }
        }
      }

      if (manifestUpdated) {
        this.state.saveManifest(manifest);
        console.log(`Saved updated manifest for file ${fileId} after repair.`);
      }
    }
  }

  // Reconstruction and relocation self-healing algorithm
  async repairShard(manifest, chunkIdx, shardIdx, fileKey) {
    const fileId = manifest.file_id;
    const k = manifest.k;
    const m = manifest.m;
    const chunkManifest = manifest.chunks[chunkIdx];

    console.log(`Self-healing: Reassembling chunk ${chunkIdx} for repair...`);

    // 1. Gather all available shards from existing holders (excluding any offline/degraded holder)
    const gatheredShards = new Array(k + m).fill(null);
    for (let idx = 0; idx < chunkManifest.shard_holders.length; idx++) {
      const holder = chunkManifest.shard_holders[idx];
      const shardHashBytes = chunkManifest.shard_hashes[idx];
      const shardHashHex = Buffer.from(shardHashBytes).toString('hex');

      if (!holder) continue;

      // Skip degraded target shard
      if (idx === shardIdx) continue;

      try {
        if (holder === this.keypair.peerId) {
          const data = this.state.readShard(shardHashHex);
          if (data) gatheredShards[idx] = data;
        } else if (this.sockets.has(holder)) {
          // Request retrieve
          const response = await this.sendRequest(holder, 'retrieve', {
            shardHash: shardHashHex
          }, null, 5000);
          
          if (response.result && response.result.success && response.payload) {
            gatheredShards[idx] = response.payload;
          }
        }
      } catch (e) {
        // Ignore failures, proceed with remaining shards
      }
    }

    const availableCount = gatheredShards.filter(s => s !== null).length;
    if (availableCount < k) {
      throw new Error(`Cannot repair chunk ${chunkIdx}: only ${availableCount} shards gathered (need at least ${k})`);
    }

    // 2. Decrypt valid shards
    const { decryptData, reconstructData, encodeData, deriveShardIV, encryptData, hashData } = await import('./core.js');
    const plainShards = new Array(k + m).fill(null);
    for (let idx = 0; idx < gatheredShards.length; idx++) {
      const enc = gatheredShards[idx];
      if (enc) {
        try {
          plainShards[idx] = decryptData(enc, fileKey);
        } catch (err) {
          console.warn(`Decryption error during repair: ${err.message}`);
        }
      }
    }

    // 3. Reed-Solomon reconstruct the plaintext chunk
    const plainChunk = reconstructData(plainShards, k, m, chunkManifest.original_len);

    // 4. Re-encode plaintext chunk to shards
    const reEncodedPlainShards = encodeData(plainChunk, k, m);

    // 5. Encrypt target missing shard
    const plainShard = reEncodedPlainShards[shardIdx];
    const iv = deriveShardIV(fileKey, chunkIdx, shardIdx);
    const encShard = encryptData(plainShard, fileKey, iv);
    const newShardHash = hashData(encShard);
    const newShardHashHex = newShardHash.toString('hex');

    // 6. Select a new healthy holder node
    // Placement constraint: must NOT hold another shard for this chunk.
    const currentHolders = new Set(chunkManifest.shard_holders.filter(h => h && h !== ''));
    
    // Potential candidates: ourselves + connected peers
    const candidates = [this.keypair.peerId, ...Array.from(this.sockets.keys())];
    let selectedPeer = null;

    for (const p of candidates) {
      if (!currentHolders.has(p)) {
        // Peer is online and doesn't hold a shard for this chunk
        selectedPeer = p;
        break;
      }
    }

    if (!selectedPeer) {
      throw new Error(`Failed to find new candidate peer for placing shard ${shardIdx} (all online nodes already hold a shard of this chunk)`);
    }

    console.log(`Repair: Selected new peer ${selectedPeer.substring(0, 8)} to hold shard ${shardIdx}`);

    // 7. Store the shard on the selected peer
    if (selectedPeer === this.keypair.peerId) {
      // Store locally
      this.state.writeShard(newShardHashHex, encShard);
    } else {
      // Send Store request
      const response = await this.sendRequest(selectedPeer, 'store', {
        shardHash: newShardHashHex
      }, encShard, 10000);
      
      if (!response.result || !response.result.success) {
        throw new Error(`Target peer ${selectedPeer} failed to store repaired shard`);
      }
    }

    // 8. Update manifest
    chunkManifest.shard_holders[shardIdx] = selectedPeer;
    // Update shard hash in manifest (should be identical, but update to be robust)
    chunkManifest.shard_hashes[shardIdx] = Array.from(newShardHash);

    console.log(`🔧 SUCCESS: Shard ${shardIdx} of chunk ${chunkIdx} successfully repaired and relocated to node ${selectedPeer.substring(0, 8)}`);
    
    this.broadcastGossip('mesh-events', {
      type: 'shard_repaired',
      file_id: fileId,
      chunk_idx: chunkIdx,
      shard_idx: shardIdx,
      new_holder: selectedPeer
    });

    return true;
  }
}
