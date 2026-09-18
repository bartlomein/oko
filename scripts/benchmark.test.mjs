import assert from 'node:assert/strict';
import test from 'node:test';
import { median, scoreResults, summarize } from './benchmark.mjs';

test('grading requires the correct file AND overlapping implementation lines', () => {
  const expected = [{ path: 'auth.rs', startLine: 100, endLine: 120,
    function: { name: 'login', startLine: 80, endLine: 130 } }];
  const wrongSection = { path: 'auth.rs', startLine: 1, endLine: 40 };
  const wrongFile = { path: 'README.md', startLine: 100, endLine: 120 };
  const correct = { path: 'auth.rs', startLine: 110, endLine: 149 };
  assert.deepEqual(scoreResults([wrongSection, wrongFile], expected), { top1: false, top5: false, functionTop1: false, functionTop5: false });
  assert.deepEqual(scoreResults([wrongSection, correct], expected), { top1: false, top5: true, functionTop1: false, functionTop5: true });
  assert.deepEqual(scoreResults([correct], expected), { top1: true, top5: true, functionTop1: true, functionTop5: true });
  assert.deepEqual(scoreResults([], expected), { top1: false, top5: false, functionTop1: false, functionTop5: false });
  assert.deepEqual(scoreResults([...Array(5).fill(wrongSection), correct], expected), { top1: false, top5: false, functionTop1: false, functionTop5: false });
  const declarationOnly = { path: 'auth.rs', startLine: 71, endLine: 90 };
  assert.deepEqual(scoreResults([declarationOnly], expected), { top1: false, top5: false, functionTop1: true, functionTop5: true });
});

test('summary keeps failed runs in the accuracy denominator but not successful timing', () => {
  const [result] = summarize([
    { mode: 'lexical', top1: true, top5: true, functionTop1: true, functionTop5: true, seconds: 2 },
    { mode: 'lexical', top1: false, top5: false, functionTop1: false, functionTop5: false, seconds: 60, error: 'timeout' },
  ]);
  assert.deepEqual(result, { mode: 'lexical', runs: 2, errors: 1, top1: 1, top5: 1,
    functionTop1: 1, functionTop5: 1, medianSeconds: 2 });
});

test('median handles single, odd and even sample counts without mutating input', () => {
  const input = [8, 2, 4];
  assert.equal(median(input), 4);
  assert.deepEqual(input, [8, 2, 4]);
  assert.equal(median([2, 8]), 5);
  assert.equal(median([3]), 3);
});
