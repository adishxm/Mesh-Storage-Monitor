import { encodeFile, decodeFile } from './core.js';

async function runTests() {
  console.log('--- Starting Core Unit Tests (Milestone 1) ---');

  // 1. Generate a mock 20MB file
  const size20mb = 20 * 1024 * 1024;
  console.log(`Generating mock ${size20mb / (1024 * 1024)}MB file...`);
  const mockFile = Buffer.alloc(size20mb);
  // Populate with a repeating pattern to avoid trivial zeros
  for (let i = 0; i < size20mb; i++) {
    mockFile[i] = i % 251;
  }

  const passphrase = 'strongpassphrase';
  const salt = 'saltsaltsalt';
  const fileId = 'test-20mb-file';
  const k = 2;
  const m = 1;

  // 2. Encode the file
  console.log('Encoding file (k=2, m=1)...');
  const startEncode = Date.now();
  const { manifest, allEncryptedShards } = await encodeFile(mockFile, passphrase, salt, fileId, k, m);
  const encodeDuration = Date.now() - startEncode;
  console.log(`Encoding completed in ${encodeDuration}ms. Created ${manifest.chunks.length} chunks.`);

  // 3. Normal decode (all shards present)
  console.log('Testing normal decode (all shards present)...');
  const startDecodeNormal = Date.now();
  const decodedNormal = await decodeFile(manifest, passphrase, salt, allEncryptedShards, k, m);
  const decodeNormalDuration = Date.now() - startDecodeNormal;
  console.log(`Normal decode completed in ${decodeNormalDuration}ms.`);
  
  if (Buffer.compare(mockFile, decodedNormal) === 0) {
    console.log('✅ PASS: Normal decode output is byte-identical!');
  } else {
    console.error('❌ FAIL: Normal decode output mismatch!');
    process.exit(1);
  }

  // 4. Missing shards test (delete any `m` shards)
  // We delete the 1st shard (index 0) of every chunk
  console.log('Testing missing shards decode (deleting 1 shard of index 0 from each chunk)...');
  const shardsMissingSome = allEncryptedShards.map(chunkShards => {
    const copy = [...chunkShards];
    copy[0] = null; // simulate missing
    return copy;
  });

  const decodedMissing = await decodeFile(manifest, passphrase, salt, shardsMissingSome, k, m);
  if (Buffer.compare(mockFile, decodedMissing) === 0) {
    console.log('✅ PASS: Missing shards decode output is byte-identical!');
  } else {
    console.error('❌ FAIL: Missing shards decode output mismatch!');
    process.exit(1);
  }

  // 5. Corrupt shard test (a flipped bit in one shard should be caught by Merkle check, discarded, and reconstructed)
  console.log('Testing corrupted shard decode (flipping 1 bit in chunk 0, shard 1)...');
  const shardsCorrupted = allEncryptedShards.map(chunkShards => chunkShards.map(s => Buffer.from(s)));
  // Flip the first byte of chunk 0, shard 1
  shardsCorrupted[0][1][0] ^= 1;

  const decodedCorrupt = await decodeFile(manifest, passphrase, salt, shardsCorrupted, k, m);
  if (Buffer.compare(mockFile, decodedCorrupt) === 0) {
    console.log('✅ PASS: Corrupted shard is successfully detected and reconstructed!');
  } else {
    console.error('❌ FAIL: Corrupt shard decode output mismatch!');
    process.exit(1);
  }

  // 6. Check that if we corrupt more than m shards, it fails
  console.log('Testing excessive corruption (flipping 1 bit in chunk 0, shard 0 AND shard 1)...');
  const shardsFailed = allEncryptedShards.map(chunkShards => chunkShards.map(s => Buffer.from(s)));
  shardsFailed[0][0][0] ^= 1;
  shardsFailed[0][1][0] ^= 1;

  try {
    await decodeFile(manifest, passphrase, salt, shardsFailed, k, m);
    console.error('❌ FAIL: Expected decode to fail due to excessive corruption, but it succeeded!');
    process.exit(1);
  } catch (err) {
    if (err.message.includes('Cannot reconstruct chunk 0')) {
      console.log('✅ PASS: Fails as expected with "Cannot reconstruct chunk 0" when available shards < k!');
    } else {
      console.error(`❌ FAIL: Failed with unexpected error: ${err.message}`);
      process.exit(1);
    }
  }

  console.log('\n🎉 ALL CORE UNIT TESTS PASSED SUCCESSFULLY! 🎉\n');
}

runTests().catch(err => {
  console.error('Unhandled test failure:', err);
  process.exit(1);
});
