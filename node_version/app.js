import express from 'express';
import cors from 'cors';
import multer from 'multer';
import path from 'path';
import fs from 'fs';
import { NodeState } from './state.js';
import { NetworkService } from './network.js';
import { encodeFile, decodeFile, deriveMasterKey, deriveFileKey } from './core.js';

// Parse command line arguments
function parseArgs() {
  const args = {
    p2pPort: 4001,
    apiPort: 3000,
    quota: 1.5,
    dialPeer: null
  };

  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '-p' || arg === '--port') {
      args.p2pPort = parseInt(argv[++i], 10);
    } else if (arg === '-a' || arg === '--api-port') {
      args.apiPort = parseInt(argv[++i], 10);
    } else if (arg === '-q' || arg === '--quota') {
      args.quota = parseFloat(argv[++i]);
    } else if (arg === '-d' || arg === '--dial') {
      args.dialPeer = argv[++i];
    }
  }
  return args;
}

async function main() {
  const args = parseArgs();
  console.log(`Starting Mesh Storage Node: P2P Port=${args.p2pPort}, API Port=${args.apiPort}, Quota=${args.quota} GB`);

  // Ensure data folder for this node exists
  const dataDir = `./data_${args.p2pPort}`;
  if (!fs.existsSync(dataDir)) {
    fs.mkdirSync(dataDir, { recursive: true });
  }


  // Custom loadOrCreateKeypair implementation
  const keyPath = path.join(dataDir, 'identity.key');
  let privateKeyBuffer;
  
  if (fs.existsSync(keyPath)) {
    privateKeyBuffer = fs.readFileSync(keyPath);
  } else {
    // Generate new Ed25519 key pair
    const { privateKey } = await import('crypto').then(c => c.generateKeyPairSync('ed25519'));
    privateKeyBuffer = privateKey.export({ type: 'pkcs8', format: 'der' });
    fs.writeFileSync(keyPath, privateKeyBuffer);
  }

  // Restore keypair
  const cryptoModule = await import('crypto');
  const privateKey = cryptoModule.createPrivateKey({
    key: privateKeyBuffer,
    format: 'der',
    type: 'pkcs8'
  });
  
  const publicKey = cryptoModule.createPublicKey(privateKey);
  const pubBytes = publicKey.export({ type: 'spki', format: 'der' });
  const rawPub = pubBytes.subarray(pubBytes.length - 32);

  // Derive libp2p-compatible PeerID starting with '12D3KooW'
  const bs58Module = await import('bs58').then(m => m.default);
  const peerIdBytes = Buffer.alloc(6 + 32);
  peerIdBytes.set([0x00, 0x24, 0x08, 0x01, 0x12, 0x20], 0);
  peerIdBytes.set(rawPub, 6);
  const peerId = bs58Module.encode(peerIdBytes);

  const keypair = { privateKey, publicKey, rawPub, peerId };
  console.log(`Local PeerID: ${peerId}`);

  // Initialize shared state
  const state = new NodeState(peerId, args.p2pPort, args.quota);

  // Initialize and start network service
  const network = new NetworkService(keypair, state);
  network.start(args.p2pPort);

  // Auto-dial peer if specified
  if (args.dialPeer) {
    console.log(`Auto-dialing bootstrap peer: ${args.dialPeer}`);
    // Wait a brief moment for P2P server to boot
    setTimeout(async () => {
      try {
        await network.pairPeer(args.dialPeer);
        console.log(`Auto-dial pairing complete.`);
      } catch (err) {
        console.error(`Failed to auto-dial bootstrap peer: ${err.message}`);
      }
    }, 1000);
  }

  // Initialize Express server
  const app = express();
  app.use(cors());
  app.use(express.json());

  // Setup multer for handling multipart file uploads in-memory
  const storage = multer.memoryStorage();
  const upload = multer({ storage: storage });

  // HTTP API routes
  app.get('/status', (req, res) => {
    res.json(state.getStatus());
  });

  app.get('/peers', (req, res) => {
    res.json(Array.from(state.connectedPeers));
  });

  app.get('/shards', (req, res) => {
    res.json(state.getStatus().shards);
  });

  app.post('/pair', async (req, res) => {
    const { multiaddr } = req.body;
    if (!multiaddr) {
      return res.status(400).send('multiaddr is required');
    }
    console.log(`Received pairing command via API for multiaddr: ${multiaddr}`);
    try {
      await network.pairPeer(multiaddr);
      res.sendStatus(200);
    } catch (err) {
      res.status(400).send(`Pairing failed: ${err.message}`);
    }
  });

  app.post('/upload', upload.single('file'), async (req, res) => {
    try {
      const fileId = req.body.file_id;
      const passphrase = req.body.passphrase;
      const salt = req.body.salt;
      const k = parseInt(req.body.k, 10) || 1;
      const m = parseInt(req.body.m, 10) || 1;
      const file = req.file;

      if (!fileId || !passphrase || !salt || !file) {
        return res.status(400).send('Missing upload fields (file_id, passphrase, salt, and file are required)');
      }

      console.log(`API Upload: File ID=${fileId}, k=${k}, m=${m}, size=${file.size} bytes`);

      // 1. Encode file (creates chunks, RS shards, encrypts, and builds Merkle DAG)
      const { manifest, allEncryptedShards } = await encodeFile(file.buffer, passphrase, salt, fileId, k, m);

      // 2. Select target nodes for storage
      // Candidates are ourselves + connected peers
      const candidates = [peerId, ...Array.from(network.sockets.keys())];
      
      if (candidates.length < k + m) {
        return res.status(400).send(`Insufficient nodes: have ${candidates.length} online, but config needs at least ${k + m}`);
      }

      // Distribute shards
      const placementPeers = candidates.slice(0, k + m);

      for (let chunkIdx = 0; chunkIdx < allEncryptedShards.length; chunkIdx++) {
        const chunkShards = allEncryptedShards[chunkIdx];
        const chunkManifest = manifest.chunks[chunkIdx];
        
        for (let shardIdx = 0; shardIdx < chunkShards.length; shardIdx++) {
          const shardData = chunkShards[shardIdx];
          const shardHash = chunkManifest.shard_hashes[shardIdx];
          const shardHashHex = Buffer.from(shardHash).toString('hex');
          const targetPeer = placementPeers[shardIdx];

          chunkManifest.shard_holders[shardIdx] = targetPeer;

          // Case A: Store locally
          if (targetPeer === peerId) {
            state.writeShard(shardHashHex, shardData);
          } 
          // Case B: Send to remote peer
          else {
            console.log(`Uploading shard ${shardIdx} of chunk ${chunkIdx} to peer ${targetPeer.substring(0, 8)}...`);
            const response = await network.sendRequest(targetPeer, 'store', {
              shardHash: shardHashHex
            }, shardData, 10000);
            
            if (!response.result || !response.result.success) {
              throw new Error(`Peer ${targetPeer} failed to store shard ${shardHashHex}`);
            }
          }
        }
      }

      // Save manifest locally
      state.saveManifest(manifest);

      // Save key locally for audits/healing
      const masterKey = await deriveMasterKey(passphrase, salt);
      const fileKey = deriveFileKey(masterKey, salt, fileId);
      state.saveFileKey(fileId, fileKey);

      // Gossip complete event
      network.broadcastGossip('mesh-events', {
        type: 'upload_complete',
        file_id: fileId,
        root_hash: manifest.root_hash
      });

      console.log(`Successfully uploaded and distributed file manifest for: ${fileId}`);
      res.json(manifest);
    } catch (err) {
      console.error(`API Upload error: ${err.message}`);
      res.status(500).send(`Upload execution failed: ${err.message}`);
    }
  });

  app.get('/download/:file_id', async (req, res) => {
    try {
      const fileId = req.params.file_id;
      const passphrase = req.query.passphrase;
      const salt = req.query.salt;
      const k = parseInt(req.query.k, 10);
      const m = parseInt(req.query.m, 10);

      if (!fileId || !passphrase || !salt || isNaN(k) || isNaN(m)) {
        return res.status(400).send('Missing query parameters (passphrase, salt, k, and m are required)');
      }

      console.log(`API Download: File ID=${fileId}, k=${k}, m=${m}`);

      // 1. Read local manifest
      const manifest = state.readManifest(fileId);
      if (!manifest) {
        return res.status(404).send(`Manifest for file ${fileId} not found locally`);
      }

      // 2. Fetch shards from holders
      const retrievedShards = [];

      for (let chunkIdx = 0; chunkIdx < manifest.chunks.length; chunkIdx++) {
        const chunkManifest = manifest.chunks[chunkIdx];
        const chunkProvidedShards = [];

        for (let shardIdx = 0; shardIdx < chunkManifest.shard_hashes.length; shardIdx++) {
          const shardHash = chunkManifest.shard_hashes[shardIdx];
          const shardHashHex = Buffer.from(shardHash).toString('hex');
          const holder = chunkManifest.shard_holders[shardIdx];

          if (!holder) {
            chunkProvidedShards.push(null);
            continue;
          }

          // Case A: Read locally
          if (holder === peerId) {
            const localData = state.readShard(shardHashHex);
            chunkProvidedShards.push(localData);
          } 
          // Case B: Request from remote peer
          else if (network.sockets.has(holder)) {
            try {
              console.log(`Retrieving shard ${shardIdx} from peer ${holder.substring(0, 8)}...`);
              const response = await network.sendRequest(holder, 'retrieve', {
                shardHash: shardHashHex
              }, null, 5000); // 5s timeout
              
              if (response.result && response.result.success && response.payload) {
                chunkProvidedShards.push(response.payload);
              } else {
                chunkProvidedShards.push(null);
              }
            } catch (err) {
              console.warn(`Failed to retrieve shard ${shardHashHex.substring(0, 8)}: ${err.message}`);
              chunkProvidedShards.push(null);
            }
          } else {
            console.warn(`Holder ${holder.substring(0, 8)} is not connected`);
            chunkProvidedShards.push(null);
          }
        }
        retrievedShards.push(chunkProvidedShards);
      }

      // 3. Decode the file (decrypts and runs Reed-Solomon reconstruction)
      const plaintext = await decodeFile(manifest, passphrase, salt, retrievedShards, k, m);

      // Gossip complete event
      network.broadcastGossip('mesh-events', {
        type: 'download_complete',
        file_id: fileId
      });

      console.log(`Successfully downloaded and reconstructed: ${fileId}`);
      
      res.setHeader('Content-Type', 'application/octet-stream');
      res.setHeader('Content-Disposition', `attachment; filename="${fileId}"`);
      res.send(plaintext);
    } catch (err) {
      console.error(`API Download error: ${err.message}`);
      res.status(500).send(`Download failed: ${err.message}`);
    }
  });

  // Start HTTP API
  app.listen(args.apiPort, '0.0.0.0', () => {
    console.log(`HTTP API server running on http://localhost:${args.apiPort}`);
  });
}

main().catch(err => {
  console.error('Failed to start Node agent:', err);
  process.exit(1);
});
