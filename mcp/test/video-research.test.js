import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { videoGroups, videoResearch, appendVideoWords, expandedSearch } from '../../desktop/src/challenge.js';

test('video evidence stays byte-identical to the reviewed source, with valid distinct words', () => {
  const source = readFileSync(new URL('../../history/research/video-onscreen-words.json', import.meta.url));
  assert.equal(createHash('sha256').update(source).digest('hex'), videoResearch.sha256);
  assert.deepEqual(videoGroups.map(g=>g.words.length), [5,109,110,192]);
  const dictionary = new Set(wordlist);
  for (const group of videoGroups) {
    assert.equal(new Set(group.words).size, group.words.length);
    assert.ok(group.words.every(word=>dictionary.has(word)));
  }
  assert.ok(videoGroups[0].words.every(word=>videoGroups[1].words.includes(word)));
  assert.equal(videoResearch.exclusion_model, null);
});

test('adding research groups preserves pins, repetitions and source order without duplicating additions', () => {
  const original = 'fog@5 parrot@12 LINK@3 fiber fiber';
  const merged = appendVideoWords(original, videoGroups[0].words);
  assert.equal(merged.added, 4);
  assert.equal(merged.text, `${original} atom basic token dash`);
  assert.deepEqual(appendVideoWords(merged.text, videoGroups[0].words), {text:merged.text,added:0});
});

test('expanded preparation preserves the original run and candidate domain but drops incompatible exclusions', () => {
  const original = {name:'A'.repeat(100),target:'target',mode:'template',pattern:'dutch ? ? ? fog ? ? ? ? ? ? parrot',pool:'fiber fork atom',post:'post',video:'video',noChecksum:false,excludeRo1:true,excludeRecords:['strict.json']};
  const prepared = expandedSearch(original);
  assert.equal(prepared.noChecksum, true);
  assert.equal(prepared.excludeRo1, false);
  assert.deepEqual(prepared.excludeRecords, []);
  assert.equal(original.noChecksum, false);
  assert.equal(original.excludeRo1, true);
  assert.deepEqual(original.excludeRecords, ['strict.json']);
  for (const key of ['target','mode','pattern','pool','post','video']) assert.equal(prepared[key], original[key]);
  assert.ok(prepared.name.length<=100);
});
