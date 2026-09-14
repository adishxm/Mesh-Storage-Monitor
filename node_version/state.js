import fs from 'fs';
import path from 'path';

export class NodeState {
  constructor(peerId, p2pPort, quotaGb) {
    this.peerId = peerId; // base58 PeerId
    this.p2pPort = p2pPort;
    this.listenAddresses = new Set();
    this.connectedPeers = new Set(); // Set of PeerIds (strings)
    this.trustedPeers = new Set(); // Set of PeerIds (strings)
    this.storageQuota = Math.round(quotaGb * 1024 * 1024 * 1024); // quota in bytes
    this.storageUsed = 0;
    this.dataDir = `./data_${p2pPort}`;

    // Ensure directories exist
    fs.mkdirSync(path.join(this.dataDir, 'shards'), { recursive: true });
    fs.mkdirSync(path.join(this.dataDir, 'manifests'), { recursive: true });
    fs.mkdirSync(path.join(this.dataDir, 'keys'), { recursive: true });

    // Always trust ourselves
    this.trustedPeers.add(peerId);

    this.loadTrustedPeers();
    this.recalculateStorageUsed();
  }

  saveFileKey(fileId, keyBuffer) {
    const filePath = path.join(this.dataDir, 'keys', `${fileId}.key`);
    try {
      fs.writeFileSync(filePath, keyBuffer);
    } catch (err) {
      console.error(`Failed to save file key for ${fileId}: ${err.message}`);
    }
  }

  readFileKey(fileId) {
    const filePath = path.join(this.dataDir, 'keys', `${fileId}.key`);
    if (fs.existsSync(filePath)) {
      try {
        return fs.readFileSync(filePath);
      } catch (err) {
        console.error(`Failed to read file key for ${fileId}: ${err.message}`);
        return null;
      }
    }
    return null;
  }

  loadTrustedPeers() {
    const filePath = path.join(this.dataDir, 'trusted_peers.json');
    if (fs.existsSync(filePath)) {
      try {
        const content = fs.readFileSync(filePath, 'utf-8');
        const list = JSON.parse(content);
        if (Array.isArray(list)) {
          for (const p of list) {
            this.trustedPeers.add(p);
          }
        }
      } catch (err) {
        console.error(`Failed to load trusted peers: ${err.message}`);
      }
    }
    console.log(`Loaded ${this.trustedPeers.size} trusted peers`);
  }

  saveTrustedPeers() {
    const filePath = path.join(this.dataDir, 'trusted_peers.json');
    try {
      const list = Array.from(this.trustedPeers);
      fs.writeFileSync(filePath, JSON.stringify(list, null, 2), 'utf-8');
    } catch (err) {
      console.error(`Failed to save trusted peers: ${err.message}`);
    }
  }

  addTrustedPeer(peerId) {
    if (!this.trustedPeers.has(peerId)) {
      this.trustedPeers.add(peerId);
      console.log(`Added trusted peer: ${peerId}`);
      this.saveTrustedPeers();
    }
  }

  isTrusted(peerId) {
    return this.trustedPeers.has(peerId);
  }

  recalculateStorageUsed() {
    let total = 0;
    const shardsDir = path.join(this.dataDir, 'shards');
    try {
      const files = fs.readdirSync(shardsDir);
      for (const file of files) {
        const filePath = path.join(shardsDir, file);
        const stat = fs.statSync(filePath);
        if (stat.isFile()) {
          total += stat.size;
        }
      }
    } catch (err) {
      console.error(`Error reading shards directory: ${err.message}`);
    }
    this.storageUsed = total;
    console.log(`Storage used: ${this.storageUsed} / ${this.storageQuota} bytes`);
  }

  hasShard(hashHex) {
    const filePath = path.join(this.dataDir, 'shards', hashHex);
    return fs.existsSync(filePath);
  }

  readShard(hashHex) {
    const filePath = path.join(this.dataDir, 'shards', hashHex);
    if (fs.existsSync(filePath)) {
      try {
        return fs.readFileSync(filePath);
      } catch (err) {
        console.error(`Error reading shard ${hashHex}: ${err.message}`);
        return null;
      }
    }
    return null;
  }

  writeShard(hashHex, data) {
    const dataLen = data.length;
    if (this.storageUsed + dataLen > this.storageQuota) {
      throw new Error('Storage quota exceeded');
    }

    const filePath = path.join(this.dataDir, 'shards', hashHex);
    try {
      fs.writeFileSync(filePath, data);
      this.recalculateStorageUsed();
    } catch (err) {
      throw new Error(`Failed to write shard: ${err.message}`);
    }
  }

  deleteShard(hashHex) {
    const filePath = path.join(this.dataDir, 'shards', hashHex);
    if (fs.existsSync(filePath)) {
      try {
        fs.unlinkSync(filePath);
        this.recalculateStorageUsed();
      } catch (err) {
        throw new Error(`Failed to delete shard: ${err.message}`);
      }
    }
  }

  saveManifest(manifest) {
    const filePath = path.join(this.dataDir, 'manifests', `${manifest.file_id}.json`);
    try {
      fs.writeFileSync(filePath, JSON.stringify(manifest, null, 2), 'utf-8');
    } catch (err) {
      throw new Error(`Failed to save manifest: ${err.message}`);
    }
  }

  readManifest(fileId) {
    const filePath = path.join(this.dataDir, 'manifests', `${fileId}.json`);
    if (fs.existsSync(filePath)) {
      try {
        const content = fs.readFileSync(filePath, 'utf-8');
        return JSON.parse(content);
      } catch (err) {
        console.error(`Failed to read manifest for ${fileId}: ${err.message}`);
        return null;
      }
    }
    return null;
  }

  listManifests() {
    const list = [];
    const manifestsDir = path.join(this.dataDir, 'manifests');
    try {
      const files = fs.readdirSync(manifestsDir);
      for (const file of files) {
        if (file.endsWith('.json')) {
          const filePath = path.join(manifestsDir, file);
          const content = fs.readFileSync(filePath, 'utf-8');
          list.push(JSON.parse(content));
        }
      }
    } catch (err) {
      console.error(`Error listing manifests: ${err.message}`);
    }
    return list;
  }

  getStatus() {
    const shardsDir = path.join(this.dataDir, 'shards');
    let shards = [];
    try {
      shards = fs.readdirSync(shardsDir);
    } catch (err) {
      console.error(`Error reading shards list: ${err.message}`);
    }

    return {
      peer_id: this.peerId,
      listen_addresses: Array.from(this.listenAddresses),
      peers: Array.from(this.connectedPeers),
      storage_used: this.storageUsed,
      storage_quota: this.storageQuota,
      shards: shards,
      trusted_peers: Array.from(this.trustedPeers),
    };
  }
}
