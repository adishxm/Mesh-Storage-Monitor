import crypto from 'crypto';
import { argon2id } from 'hash-wasm';
import bs58 from 'bs58';

// ==========================================
// 1. GALOIS FIELD GF(256) ARITHMETIC
// ==========================================
const PRIMITIVE = 0x11d; // 285, standard primitive polynomial for RS in GF(256)
const gfExp = new Uint8Array(512);
const gfLog = new Uint8Array(256);

// Initialize exp and log tables
let x = 1;
for (let i = 0; i < 255; i++) {
  gfExp[i] = x;
  gfLog[x] = i;
  x <<= 1;
  if (x & 0x100) {
    x ^= PRIMITIVE;
  }
}
for (let i = 255; i < 512; i++) {
  gfExp[i] = gfExp[i - 255];
}

export function gfAdd(a, b) {
  return a ^ b;
}

export function gfMul(a, b) {
  if (a === 0 || b === 0) return 0;
  return gfExp[gfLog[a] + gfLog[b]];
}

export function gfDiv(a, b) {
  if (b === 0) throw new Error('Division by zero in GF(256)');
  if (a === 0) return 0;
  let diff = gfLog[a] - gfLog[b];
  if (diff < 0) diff += 255;
  return gfExp[diff];
}

// ==========================================
// 2. MATRIX OPERATIONS & GAUSSIAN ELIMINATION
// ==========================================
export function invertMatrix(matrix, k) {
  // matrix is a 2D array of size k x k
  // Create an augmented matrix [A | I]
  const aug = [];
  for (let i = 0; i < k; i++) {
    aug[i] = new Uint8Array(2 * k);
    for (let j = 0; j < k; j++) {
      aug[i][j] = matrix[i][j];
    }
    aug[i][k + i] = 1;
  }

  for (let i = 0; i < k; i++) {
    // Find pivot row
    let pivotRow = i;
    while (pivotRow < k && aug[pivotRow][i] === 0) {
      pivotRow++;
    }
    if (pivotRow === k) {
      throw new Error('Matrix is singular and cannot be inverted');
    }
    // Swap rows if necessary
    if (pivotRow !== i) {
      const temp = aug[i];
      aug[i] = aug[pivotRow];
      aug[pivotRow] = temp;
    }

    // Scale pivot row so that pivot element is 1
    const pivotVal = aug[i][i];
    if (pivotVal !== 1) {
      const invPivot = gfDiv(1, pivotVal);
      for (let j = i; j < 2 * k; j++) {
        aug[i][j] = gfMul(aug[i][j], invPivot);
      }
    }

    // Eliminate pivot column elements in all other rows
    for (let r = 0; r < k; r++) {
      if (r !== i) {
        const factor = aug[r][i];
        if (factor !== 0) {
          for (let j = i; j < 2 * k; j++) {
            aug[r][j] = gfAdd(aug[r][j], gfMul(aug[i][j], factor));
          }
        }
      }
    }
  }

  // Extract inverted matrix
  const inv = [];
  for (let i = 0; i < k; i++) {
    inv[i] = new Uint8Array(k);
    for (let j = 0; j < k; j++) {
      inv[i][j] = aug[i][k + j];
    }
  }
  return inv;
}

// ==========================================
// 3. CAUCHY REED-SOLOMON ERASURE CODING
// ==========================================
// Generates a Cauchy matrix coefficients of size m x k
function getCauchyMatrix(k, m) {
  const matrix = [];
  for (let i = 0; i < m; i++) {
    matrix[i] = new Uint8Array(k);
    for (let j = 0; j < k; j++) {
      // Cauchy element: 1 / (x_i ^ y_j) where x_i = k + i, y_j = j
      // This guarantees any submatrix is invertible
      matrix[i][j] = gfDiv(1, (k + i) ^ j);
    }
  }
  return matrix;
}

// Encodes data into k data + m parity shards
export function encodeData(data, k, m) {
  if (k <= 0 || m <= 0) {
    throw new Error(`Invalid Reed-Solomon parameters: k=${k}, m=${m}`);
  }

  const dataLen = data.length;
  const shardSize = Math.ceil(dataLen / k);
  const paddedLen = shardSize * k;

  // Padded buffer
  const paddedData = new Uint8Array(paddedLen);
  paddedData.set(data);

  // Initialize shards array
  const shards = [];
  for (let j = 0; j < k; j++) {
    shards.push(paddedData.subarray(j * shardSize, (j + 1) * shardSize));
  }

  // Generate Cauchy parity shards
  const cauchy = getCauchyMatrix(k, m);
  for (let i = 0; i < m; i++) {
    const parityShard = new Uint8Array(shardSize);
    for (let p = 0; p < shardSize; p++) {
      let sum = 0;
      for (let j = 0; j < k; j++) {
        sum = gfAdd(sum, gfMul(cauchy[i][j], shards[j][p]));
      }
      parityShard[p] = sum;
    }
    shards.push(parityShard);
  }

  return shards; // Returns k data shards + m parity shards
}

// Reconstructs data from any k of the k+m shards
export function reconstructData(shards, k, m, originalLen) {
  if (k <= 0) {
    throw new Error(`Invalid Reed-Solomon parameter: k=${k}`);
  }

  // Check how many shards are available
  const availableIndices = [];
  const availableShards = [];
  for (let idx = 0; idx < shards.length; idx++) {
    if (shards[idx]) {
      availableIndices.push(idx);
      availableShards.push(shards[idx]);
    }
  }

  if (availableIndices.length < k) {
    throw new Error(`Too few shards available: got ${availableIndices.length}, need ${k}`);
  }

  // Take the first k available shards
  const subIndices = availableIndices.slice(0, k);
  const subShards = availableShards.slice(0, k);
  const shardSize = subShards[0].length;

  // Construct Generator Matrix G of size (k + m) x k
  // Row j (j < k) is identity matrix
  // Row j (j >= k) is Cauchy matrix row (j - k)
  const cauchy = getCauchyMatrix(k, m);
  const G = [];
  for (let r = 0; r < k + m; r++) {
    G[r] = new Uint8Array(k);
    if (r < k) {
      G[r][r] = 1;
    } else {
      G[r].set(cauchy[r - k]);
    }
  }

  // Construct submatrix A (k x k) by choosing rows matching the available shards
  const A = [];
  for (let r = 0; r < k; r++) {
    A[r] = G[subIndices[r]];
  }

  // Invert submatrix A
  const A_inv = invertMatrix(A, k);

  // Reconstruct the first k shards (original data shards)
  const reconstructedData = new Uint8Array(k * shardSize);
  for (let p = 0; p < shardSize; p++) {
    for (let r = 0; r < k; r++) {
      let sum = 0;
      for (let c = 0; c < k; c++) {
        sum = gfAdd(sum, gfMul(A_inv[r][c], subShards[c][p]));
      }
      reconstructedData[r * shardSize + p] = sum;
    }
  }

  // Truncate original length
  if (reconstructedData.length > originalLen) {
    return reconstructedData.slice(0, originalLen);
  }
  return reconstructedData;
}

// ==========================================
// 4. FASTCDC CONTENT-DEFINED CHUNKING
// ==========================================
// Deterministic pseudo-random Gear hash table derived from a seed to be cross-platform
const GEAR = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let hash = i;
  for (let j = 0; j < 5; j++) {
    hash = Math.imul(hash ^ 0xcc9e2d51, 0x1b873593);
    hash = (hash << 15) | (hash >>> 17);
  }
  GEAR[i] = hash;
}

export function chunkData(data) {
  // Standard params for ~2MB average chunk size
  const minSize = 524288;     // 512 KB
  const avgSize = 2097152;   // 2 MB
  const maxSize = 8388608;   // 8 MB

  const chunks = [];
  const len = data.length;
  if (len === 0) return chunks;
  if (len <= minSize) {
    chunks.push(new Uint8Array(data));
    return chunks;
  }

  // Bits determination
  const bits = Math.round(Math.log2(avgSize)); // 21
  const maskS = (1 << (bits + 1)) - 1; // 22 bits
  const maskL = (1 << (bits - 1)) - 1; // 20 bits

  let offset = 0;
  while (offset < len) {
    if (len - offset <= minSize) {
      chunks.push(data.subarray(offset));
      break;
    }

    let chunkLen = minSize;
    let hash = 0;
    
    while (offset + chunkLen < len && chunkLen < maxSize) {
      const b = data[offset + chunkLen];
      hash = ((hash << 1) >>> 0) + GEAR[b];
      
      const mask = chunkLen < (avgSize / 2) ? maskS : maskL;
      if ((hash & mask) === 0) {
        chunkLen++;
        break;
      }
      chunkLen++;
    }
    
    chunks.push(data.subarray(offset, offset + chunkLen));
    offset += chunkLen;
  }
  return chunks;
}

// ==========================================
// 5. CRYPTO UTILS & KEY DERIVATION
// ==========================================
export async function deriveMasterKey(passphrase, salt) {
  // Argon2id parameters: 16MB memory (16384 KB), 1 iteration, 1 parallelism.
  // Returns 32-byte key as a Buffer.
  const passBytes = typeof passphrase === 'string' ? Buffer.from(passphrase) : passphrase;
  const saltBytes = typeof salt === 'string' ? Buffer.from(salt) : salt;

  const result = await argon2id({
    password: passBytes,
    salt: saltBytes,
    parallelism: 1,
    memorySize: 16384, // 16MB
    iterations: 1,
    hashLength: 32,
    outputType: 'binary'
  });
  return Buffer.from(result);
}

export function deriveFileKey(masterKey, salt, fileId) {
  const saltBytes = typeof salt === 'string' ? Buffer.from(salt) : salt;
  const infoBytes = typeof fileId === 'string' ? Buffer.from(fileId) : fileId;
  return Buffer.from(crypto.hkdfSync('sha256', masterKey, saltBytes, infoBytes, 32));
}

export function deriveShardIV(fileKey, chunkIdx, shardIdx) {
  const info = `iv-${chunkIdx}-${shardIdx}`;
  return Buffer.from(crypto.hkdfSync('sha256', fileKey, Buffer.alloc(0), info, 12));
}

// Encrypts data using AES-256-GCM
// Returns a Buffer payload with layout: [12-byte IV] + [16-byte Auth Tag] + [ciphertext]
export function encryptData(data, key, iv) {
  const cipher = crypto.createCipheriv('aes-256-gcm', key, iv);
  const ciphertext = Buffer.concat([cipher.update(data), cipher.final()]);
  const tag = cipher.getAuthTag(); // 16 bytes
  return Buffer.concat([iv, tag, ciphertext]);
}

// Decrypts data from a payload structured as: [12-byte IV] + [16-byte Auth Tag] + [ciphertext]
export function decryptData(payload, key) {
  if (payload.length < 28) {
    throw new Error(`Invalid payload size: expected at least 28 bytes, got ${payload.length}`);
  }
  const iv = payload.subarray(0, 12);
  const tag = payload.subarray(12, 28);
  const ciphertext = payload.subarray(28);

  const decipher = crypto.createDecipheriv('aes-256-gcm', key, iv);
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

// ==========================================
// 6. INTEGRITY & MERKLE UTILS
// ==========================================
export function hashData(data) {
  return crypto.createHash('sha256').update(data).digest();
}

export function computeChunkHash(shardHashes) {
  const hasher = crypto.createHash('sha256');
  for (const hash of shardHashes) {
    hasher.update(hash);
  }
  return hasher.digest();
}

export function computeRootHash(chunkHashes) {
  const hasher = crypto.createHash('sha256');
  for (const hash of chunkHashes) {
    hasher.update(hash);
  }
  return hasher.digest();
}

export function verifyShard(shardData, expectedHash) {
  const hash = hashData(shardData);
  return Buffer.compare(hash, expectedHash) === 0;
}

// ==========================================
// 7. HIGH-LEVEL API: ENCODE / DECODE FILE
// ==========================================
export async function encodeFile(data, passphrase, salt, fileId, k, m) {
  const masterKey = await deriveMasterKey(passphrase, salt);
  const fileKey = deriveFileKey(masterKey, salt, fileId);

  const dataBuffer = Buffer.isBuffer(data) ? data : Buffer.from(data);
  const chunks = chunkData(new Uint8Array(dataBuffer));
  const chunkManifests = [];
  const allEncryptedShards = [];
  const chunkHashes = [];

  for (let chunkIdx = 0; chunkIdx < chunks.length; chunkIdx++) {
    const chunk = chunks[chunkIdx];
    const originalChunkLen = chunk.length;

    // Reed-Solomon encode plaintext chunk
    const plainShards = encodeData(chunk, k, m);

    const audits = [];
    const encryptedShards = [];
    const shardHashes = [];
    for (let shardIdx = 0; shardIdx < plainShards.length; shardIdx++) {
      const shard = plainShards[shardIdx];
      const iv = deriveShardIV(fileKey, chunkIdx, shardIdx);
      const enc = encryptData(shard, fileKey, iv);
      const hash = hashData(enc);

      encryptedShards.push(enc);
      shardHashes.push(hash);

      // Pre-compute 5 audits
      const shardAudits = [];
      for (let c = 0; c < 5; c++) {
        const nonce = crypto.randomBytes(16);
        const hashInput = Buffer.concat([enc, nonce]);
        const response = crypto.createHash('sha256').update(hashInput).digest('hex');
        shardAudits.push({
          nonce: nonce.toString('hex'),
          response: response
        });
      }
      audits.push(shardAudits);
    }

    const chunkHash = computeChunkHash(shardHashes);
    chunkHashes.push(chunkHash);

    chunkManifests.push({
      chunk_hash: Array.from(chunkHash), // serialized as array of 32 bytes
      shard_hashes: shardHashes.map(h => Array.from(h)),
      shard_holders: new Array(k + m).fill(''),
      original_len: originalChunkLen,
      audits: audits
    });
    allEncryptedShards.push(encryptedShards);
  }

  const rootHash = computeRootHash(chunkHashes);
  const manifest = {
    file_id: fileId,
    original_len: dataBuffer.length,
    root_hash: Array.from(rootHash),
    k: k,
    m: m,
    chunks: chunkManifests
  };

  return { manifest, allEncryptedShards };
}

export async function decodeFile(manifest, passphrase, salt, shards, k, m) {
  const masterKey = await deriveMasterKey(passphrase, salt);
  const fileKey = deriveFileKey(masterKey, salt, manifest.file_id);

  const reassembledFileParts = [];

  if (shards.length !== manifest.chunks.length) {
    throw new Error(`Chunk count mismatch: manifest has ${manifest.chunks.length}, provided ${shards.length}`);
  }

  for (let chunkIdx = 0; chunkIdx < manifest.chunks.length; chunkIdx++) {
    const chunkManifest = manifest.chunks[chunkIdx];
    const providedShards = shards[chunkIdx];

    if (providedShards.length !== k + m) {
      throw new Error(`Shard count mismatch for chunk ${chunkIdx}: expected ${k + m}, got ${providedShards.length}`);
    }

    // 1. Verify shard integrity using the Merkle DAG hashes.
    // If a shard fails verification, treat it as missing (null) to trigger reconstruction.
    const verifiedEncryptedShards = [];
    for (let shardIdx = 0; shardIdx < providedShards.length; shardIdx++) {
      const optShard = providedShards[shardIdx];
      const expectedHash = Buffer.from(chunkManifest.shard_hashes[shardIdx]);

      if (optShard) {
        if (verifyShard(optShard, expectedHash)) {
          verifiedEncryptedShards.push(Buffer.isBuffer(optShard) ? optShard : Buffer.from(optShard));
        } else {
          // Integrity check failed: discard shard!
          verifiedEncryptedShards.push(null);
        }
      } else {
        verifiedEncryptedShards.push(null);
      }
    }

    // 2. Count valid shards
    const validCount = verifiedEncryptedShards.filter(s => s !== null).length;
    if (validCount < k) {
      throw new Error(`Cannot reconstruct chunk ${chunkIdx}: only ${validCount} valid shards available (need at least ${k})`);
    }

    // 3. Decrypt the valid shards
    const plainShards = [];
    for (let shardIdx = 0; shardIdx < verifiedEncryptedShards.length; shardIdx++) {
      const optEncShard = verifiedEncryptedShards[shardIdx];
      if (optEncShard) {
        const dec = decryptData(optEncShard, fileKey);
        plainShards.push(dec);
      } else {
        plainShards.push(null);
      }
    }

    // 4. Reed-Solomon reconstruct the chunk
    const reconstructedChunk = reconstructData(
      plainShards,
      k,
      m,
      chunkManifest.original_len
    );

    reassembledFileParts.push(reconstructedChunk);
  }

  return Buffer.concat(reassembledFileParts);
}

// ==========================================
// 8. IDENTITY & HANDSHAKE CRYPTO
// ==========================================
export function getPublicKeyFromPeerId(peerId) {
  const bytes = bs58.decode(peerId);
  if (bytes.length !== 38 || bytes[0] !== 0x00 || bytes[1] !== 0x24) {
    throw new Error('Invalid PeerId format');
  }
  return bytes.subarray(6); // 32 bytes raw public key
}

export function verifySignature(publicKeyBytes, data, signature) {
  const key = crypto.createPublicKey({
    key: Buffer.concat([
      Buffer.from([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]), // SPKI prefix for Ed25519
      publicKeyBytes
    ]),
    format: 'der',
    type: 'spki'
  });
  return crypto.verify(null, data, key, signature);
}

export function signData(privateKey, data) {
  return crypto.sign(null, data, privateKey);
}
