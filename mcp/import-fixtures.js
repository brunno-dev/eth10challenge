// Independent, disposable fixtures for desktop queue integration tests.
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { ECDH } from 'node:crypto';
import { HDKey } from '@scure/bip32';
import { validateMnemonic, mnemonicToSeedSync } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { keccak_256 } from '@noble/hashes/sha3.js';
import assert from 'node:assert/strict';

export function address(phrase) {
  const key=HDKey.fromMasterSeed(mnemonicToSeedSync(phrase)).derive("m/44'/60'/0'/0/0");
  const publicKey=ECDH.convertKey(key.publicKey,'secp256k1',undefined,undefined,'uncompressed');
  return '0x'+Buffer.from(keccak_256(publicKey.subarray(1)).subarray(12)).toString('hex');
}
assert.equal(address('abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'),'0x9858effd232b4033e47d90003d41ec34ecaeda94');
let phrase;
for(const word of wordlist) {
  const candidate=`dutch abandon abandon abandon fog abandon abandon abandon abandon abandon ${word} parrot`;
  if(validateMnemonic(candidate,wordlist)){phrase=candidate;break;}
}
assert(phrase);
const directory=new URL('../output/import-check/',import.meta.url);
await mkdir(directory,{recursive:true});
const first='parrot abandon abandon dutch abandon abandon abandon fog abandon abandon abandon abandon';
const negative=[first,first.split(' ').reverse().join(' '),first.replace('abandon','ability'),'dutch fog parrot',first.replace('fog','cloud')].join('\n');
const success=phrase.split(' ').reverse().join(' ')+'\n'+first.replaceAll('abandon','ability');
const long='dutch fiber fork dinner cloud live fog wood winter rib parrot lake';
await writeFile(new URL('negative.txt',directory),negative);
await writeFile(new URL('success.txt',directory),success);
await writeFile(new URL('pause.txt',directory),long+'\n'+long.replace('lake','also'));
await writeFile(new URL('fixture.json',directory),JSON.stringify({target:address(phrase),phrase,negative,success,long},null,2));
console.log(JSON.stringify({directory:fileURLToPath(directory),successTarget:address(phrase)}));
