import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import {
  root, reference, parityBinary, requireArtifacts, runProcess, runCli, runJson,
  assertParity, implementationSnapshot,
} from './runtime-utils.mjs';

async function main() {
  if (process.argv.length > 2) throw new Error('Usage: node scripts/parity-runtimes.mjs');
  await requireArtifacts({ diagnostic: true });
  const requireReference = createRequire(resolve(reference, 'package.json'));
  const search = await import(pathToFileURL(resolve(reference, 'dist/search.js')));
  const { rankItems } = await import(pathToFileURL(resolve(reference, 'dist/rank-items.js')));
  const { stemmer } = await import(pathToFileURL(requireReference.resolve('stemmer')));
  const dotenv = requireReference('dotenv');
  const startedAt = new Date().toISOString();
  const before = await implementationSnapshot({ diagnostic: true });
  const checks = [];

  async function checkDiagnostic(id, fixture) {
    const chunks = (fixture.files ?? []).flatMap(file => search.chunkText(file.path, file.text));
    const expected = { chunks, shortlist: search.rankLexically(chunks, fixture.question),
      request: null, omittedCount: 0, stems: (fixture.words ?? []).map(stemmer) };
    if (fixture.dotenv !== undefined) expected.dotenvValues = dotenv.parse(fixture.dotenv);
    let referenceError;
    try {
      if (fixture.items !== undefined) {
        const result = await rankItems(fixture.question, fixture.items, { apiKey: 'local-parity-test', limit: fixture.limit,
          clientFactory: () => ({ systemOne: async request => {
            expected.request = JSON.parse(JSON.stringify(request));
            if (Object.hasOwn(fixture, 'response')) return fixture.response;
            const probabilities = { none: 0 };
            request.state.candidates.forEach(item => { probabilities[item.candidate] = 0; });
            return { answers: { selection: { probabilities } } };
          } }) });
        expected.omittedCount = result.omittedCount;
        if (Object.hasOwn(fixture, 'response')) expected.ranking = result;
      }
    } catch (error) { referenceError = error; }
    const actual = await runProcess(parityBinary, [], { input: JSON.stringify(fixture) });
    if (referenceError) {
      if (actual.code === 0) throw new Error(`Runtime parity failed: ${id}; TypeScript rejected, Rust accepted`);
      checks.push({ id, kind: 'diagnostic-rejection', passed: true });
      return;
    }
    if (actual.code !== 0) throw new Error(`Diagnostic failed: ${id}: ${actual.stderr.trim()}`);
    assertParity(JSON.parse(actual.stdout), expected, id);
    checks.push({ id, kind: 'chunks-shortlist-request-stems', passed: true,
      chunks: chunks.length, stems: expected.stems.length, omittedCount: expected.omittedCount });
  }

  const textFiles = [
    { path: 'src/camel.ts', text: 'export async function refreshAccessToken() {\r\n  return HTTPAccessToken;\r\n}\r\n' },
    { path: 'src/snake.rs', text: '/// Writes atomically, then renames.\n#[cfg(test)]\npub(crate) async fn save_project() {\n let temporary_file = write();\n rename();\n}\n' },
    { path: 'src/python.py', text: '# Sensor records\rasync def parse_sensor():\r    return telemetry_points\r' },
    { path: 'src/éclair.ts', text: 'const access_token = "é😀";\n' },
    { path: 'src/😀.ts', text: 'const access_token = true;\n' },
    { path: 'src/\uE000.ts', text: 'const access_token = true;\n' },
    { path: 'src/long.js', text: ['// Processes imported records.', 'export async function processRecords() {',
      ...Array.from({ length: 301 }, (_, index) => `  consume(record_${index});`), '}',
      'function after() { return save_project(); }'].join('\n') },
    { path: 'notes.md', text: Array.from({ length: 121 }, (_, index) => `Line ${index + 1}: saved records and tokens.`).join('\r\n') },
    { path: 'src/empty.rs', text: '' },
    { path: 'src/blank.py', text: '\n\n' },
    ...Array.from({ length: 36 }, (_, index) => ({ path: `ties/${String(index).padStart(2, '0')}.ts`, text: 'const access_token = true;' })),
  ];
  for (const [id, question] of [
    ['camel-snake-and-ties', 'where is access token refreshed?'],
    ['word-forms', 'Where is it saved atomically by writing and renaming?'],
    ['function-boundaries', 'processing imported records'],
    ['stop-words', 'where is it by the'],
    ['no-match', 'unfindablewordxyz'],
  ]) await checkDiagnostic(id, { files: textFiles, question });

  const baseWords = ['a', 'by', 'sky', 'y', 'yy', 'yyy', '123', 'save', 'rename', 'atomic', 'write', 'parse',
    'refresh', 'access', 'token', 'authorize', 'session', 'process', 'relate', 'happy', 'hope', 'agree',
    'caress', 'poni', 'cat', 'feed', 'bled', 'motoring', 'sing', 'conflat', 'trouble', 'size', 'hop',
    'tan', 'fall', 'hiss', 'fail', 'file', 'triplicate', 'formal', 'electric', 'good', 'revival', 'allow',
    'inference', 'airliner', 'adjustable', 'defensible', 'irritant', 'replacement', 'adoption', 'homologous'];
  const suffixes = ['', 's', 'ss', 'sses', 'ies', 'eed', 'ed', 'ing', 'y', 'ational', 'tional', 'enci', 'anci',
    'izer', 'bli', 'alli', 'entli', 'eli', 'ousli', 'ization', 'ation', 'ator', 'alism', 'iveness', 'fulness',
    'ousness', 'aliti', 'iviti', 'biliti', 'logi', 'icate', 'ative', 'alize', 'iciti', 'ical', 'ful', 'ness',
    'al', 'ance', 'ence', 'er', 'ic', 'able', 'ible', 'ant', 'ement', 'ment', 'ent', 'ou', 'ism', 'ate',
    'iti', 'ous', 'ive', 'ize', 'ion', 'e', 'll'];
  let seed = 0x6f6b6f;
  const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed; };
  const alphabet = 'abcdefghijklmnopqrstuvwxyz0123456789';
  const randomWords = Array.from({ length: 5000 }, () => {
    const length = 1 + random() % 20;
    return Array.from({ length }, () => alphabet[random() % alphabet.length]).join('');
  });
  const words = [...new Set([...baseWords.flatMap(word => suffixes.map(suffix => word + suffix)), ...randomWords])];
  await checkDiagnostic('stemmer-corpus', { question: 'stemming', words });

  const tickets = JSON.parse(await readFile(resolve(root, 'benchmarks/support-tickets.json'), 'utf8'));
  await checkDiagnostic('generic-request', { question: 'Which ticket needs a refund?', items: tickets.items });
  await checkDiagnostic('reserved-ids-and-private-columns', { question: 'refund', items: [
    { id: 'none', text: 'Invoice', source: 'row/4', secretColumn: 'must be discarded' },
    { id: 'candidate_1', text: 'Refund 😀', source: '' },
  ] });
  await checkDiagnostic('empty-items', { question: 'refund', items: [] });
  const budgetItems = Array.from({ length: 30 }, (_, index) => ({ id: `row-${index}`,
    text: `Refund ${index} ${'é😀\\\"\n'.repeat(450)}`, source: `support/é/${index}` }));
  await checkDiagnostic('payload-budget-utf8', { question: 'refund 😀', items: budgetItems });
  // Exercise inclusion on either side of the exact serialized-byte boundary.
  for (const length of [30_800, 31_200, 31_600, 32_000]) {
    await checkDiagnostic(`payload-first-item-${length}`, { question: 'refund',
      items: [{ id: 'first', text: 'x'.repeat(length) }, { id: 'second', text: 'Refund' }] });
  }
  const shortlist = search.rankLexically(textFiles.flatMap(file => search.chunkText(file.path, file.text)), 'saved records token');
  await checkDiagnostic('code-shortlist-request', { question: 'saved records token', items: shortlist.map((chunk, index) => ({
    id: String(index), text: chunk.text, source: `${chunk.path}:${chunk.startLine}-${chunk.endLine}`,
  })) });
  const responseItems = [{ id: 'none', text: 'Refund one' }, { id: 'candidate_1', text: 'Refund two', source: 'tickets/2' },
    { id: '3', text: 'Invoice' }];
  for (const [id, probabilities, limit] of [
    ['rank-tie-order', { none: 0.1, candidate_1: 0.6, candidate_2: 0.6, candidate_3: 0.05 }, 5],
    ['rank-limit', { none: 0.1, candidate_1: 0.6, candidate_2: 0.8, candidate_3: 0.2 }, 1],
    ['rank-none-cutoff', { none: 0.6, candidate_1: 0.6, candidate_2: 0.3, candidate_3: 0.1 }, 5],
    ['rank-missing-none', { candidate_1: 0.6, candidate_2: 0.3, candidate_3: 0.1 }, 5],
    ['rank-missing-candidate', { none: 0.1, candidate_1: 0.6, candidate_3: 0.1 }, 5],
    ['rank-invalid-probability', { none: 0.1, candidate_1: 1.1, candidate_2: 0.3, candidate_3: 0.1 }, 5],
  ]) await checkDiagnostic(id, { question: 'refund', items: responseItems, limit,
    response: { answers: { selection: { probabilities } } } });
  for (const [id, text] of [
    ['dotenv-literal-values', 'TYPESAFE_API_KEY=$NOT_EXPANDED\nOTHER="${ALSO_NOT_EXPANDED}"\n'],
    ['dotenv-comments-and-duplicates', 'export TYPESAFE_API_KEY=old\r\nnot a declaration\r\nTYPESAFE_API_KEY="new # quoted" # comment\nCOLON: yes\n'],
    ['dotenv-quotes-newlines-empty', 'SINGLE=\'a\\nb\'\nDOUBLE="a\\nb\\rc"\nBACKTICK=`a\\nb`\nEMPTY=\nSPACE=  abc  \n'],
    ['dotenv-whitespace', '\uFEFFKEY=good\n\u0085OTHER=bad\nDOT.KEY=dot\nDASH-KEY=dash\n'],
  ]) await checkDiagnostic(id, { question: 'dotenv', dotenv: text });

  const temporary = await mkdtemp(join(tmpdir(), 'oko-runtime-parity-'));
  try {
    await mkdir(join(temporary, '.git'));
    await writeFile(join(temporary, '.gitignore'), 'ignored.ts\nignored-dir/\n');
    for (const file of textFiles) {
      const absolute = join(temporary, file.path);
      await mkdir(resolve(absolute, '..'), { recursive: true });
      await writeFile(absolute, file.text);
    }
    await mkdir(join(temporary, 'ignored-dir'));
    await writeFile(join(temporary, 'ignored.ts'), 'const forbidden = "access_token";');
    await writeFile(join(temporary, 'ignored-dir/hidden.ts'), 'const forbidden = "access_token";');
    await writeFile(join(temporary, '.hidden.ts'), 'const forbidden = "access_token";');
    await writeFile(join(temporary, 'binary.dat'), Buffer.from('access_token\0forbidden'));
    await writeFile(join(temporary, 'invalid.dat'), Buffer.from([0x61, 0x63, 0x63, 0xc3, 0x28]));
    await writeFile(join(temporary, 'oversize.txt'), Buffer.alloc(256 * 1024 + 1, 'access_token '));
    await writeFile(join(temporary, 'boundary-size.txt'), Buffer.alloc(256 * 1024, 'boundary_size '));
    await writeFile(join(temporary, 'bom.ts'), Buffer.from('\uFEFFconst access_token = true;\n'));
    for (const question of ['access token', 'saved atomically writing renaming', 'processing records',
      'forbidden', 'boundary size', 'where is it by the', 'unfindablewordxyz']) {
      const args = ['ask', question, '--no-jev', '--json'];
      const ts = await runJson('typescript', args, temporary);
      const rust = await runJson('rust', args, temporary);
      assertParity(rust.value, ts.value, `filesystem CLI: ${question}`);
      checks.push({ id: `filesystem:${question}`, kind: 'cli-exact-json', passed: true });
    }
    for (const [id, question] of [['trim-bom', '\uFEFF access token \uFEFF'], ['preserve-next-line', '\u0085access token\u0085']]) {
      const args = ['ask', question, '--no-jev', '--json'];
      const ts = await runJson('typescript', args, temporary);
      const rust = await runJson('rust', args, temporary);
      assertParity(rust.value, ts.value, `argument whitespace: ${id}`);
      checks.push({ id, kind: 'cli-exact-json', passed: true });
    }

    const inputCases = [
      ['tickets', tickets.items], ['empty', []],
      ['extra-fields', [{ id: 'x', text: 'refund', privateColumn: 'discarded' }]],
      ['maximum-items', Array.from({ length: 30 }, (_, index) => ({ id: String(index), text: 'refund' }))],
      ['unicode-id-boundary', [{ id: '😀'.repeat(100), text: 'refund' }]],
      ['invalid-unicode-id-length', [{ id: '😀'.repeat(101), text: 'refund' }]],
      ['invalid-source', [{ id: 'x', text: 'refund', source: null }]],
      ['invalid-duplicate', [{ id: 'x', text: 'one' }, { id: 'x', text: 'two' }]],
      ['invalid-empty-text', [{ id: 'x', text: '  ' }]],
      ['invalid-empty-id', [{ id: '\uFEFF', text: 'refund' }]],
      ['invalid-maximum-items', Array.from({ length: 31 }, (_, index) => ({ id: String(index), text: 'refund' }))],
      ['invalid-not-array', { id: 'x', text: 'refund' }],
    ];
    const inputPath = join(temporary, '.items.json');
    for (const [id, items] of inputCases) {
      await writeFile(inputPath, JSON.stringify(items));
      const args = ['rank', '--input', inputPath, 'refund', '--no-jev', '--json'];
      const ts = await runCli('typescript', args, temporary);
      const rust = await runCli('rust', args, temporary);
      assertParity(rust.code === 0, ts.code === 0, `generic input accepted/rejected: ${id}`);
      if (ts.code === 0) assertParity(JSON.parse(rust.stdout), JSON.parse(ts.stdout), `generic input result: ${id}`);
      checks.push({ id: `input:${id}`, kind: ts.code === 0 ? 'cli-exact-json' : 'cli-rejection', passed: true });
    }
    for (const [id, text] of [['invalid-json', '{'], ['input-size-limit', ' '.repeat(1024 * 1024 + 1)]]) {
      await writeFile(inputPath, text);
      for (const runtime of ['typescript', 'rust']) {
        const result = await runCli(runtime, ['rank', '--input', inputPath, 'refund', '--no-jev', '--json'], temporary);
        if (result.code === 0) throw new Error(`${runtime} incorrectly accepted ${id}`);
      }
      checks.push({ id, kind: 'cli-rejection', passed: true });
    }
    await writeFile(inputPath, JSON.stringify(tickets.items));
    for (const [id, args] of [
      ['code-human', ['ask', 'access token', '--no-jev']],
      ['items-human', ['rank', '--input', inputPath, 'refund', '--no-jev']],
      ['empty-human', ['ask', 'unfindablewordxyz', '--no-jev']],
    ]) {
      const ts = await runCli('typescript', args, temporary);
      const rust = await runCli('rust', args, temporary);
      assertParity({ code: rust.code, stdout: rust.stdout, stderr: rust.stderr },
        { code: ts.code, stdout: ts.stdout, stderr: ts.stderr }, id);
      checks.push({ id, kind: 'cli-exact-text', passed: true });
    }
    for (const [id, dotenv] of [
      ['env-basic', 'TYPESAFE_API_KEY=local-test-key\n'],
      ['env-quotes', 'export TYPESAFE_API_KEY="local-test-key" # comment\n'],
      ['env-unrelated-malformed-line', 'TYPESAFE_API_KEY=local-test-key\nnot a valid declaration\n'],
      ['env-colon-syntax', 'TYPESAFE_API_KEY: local-test-key\n'],
    ]) {
      await writeFile(join(temporary, '.env'), dotenv);
      const args = ['ask', 'access token', '--no-jev', '--json'];
      const ts = await runCli('typescript', args, temporary);
      const rust = await runCli('rust', args, temporary);
      assertParity(rust.code === 0, ts.code === 0, id);
      if (ts.code === 0) assertParity(JSON.parse(rust.stdout), JSON.parse(ts.stdout), id);
      checks.push({ id, kind: 'dotenv-cli-parity', passed: true });
    }
  } finally { await rm(temporary, { recursive: true, force: true }); }
  assertParity(await implementationSnapshot({ diagnostic: true }), before, 'implementation snapshots before/after parity run');
  const directory = resolve(root, 'benchmarks/results', `parity-${startedAt.replaceAll(':', '-')}`);
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, 'report.json'), JSON.stringify({ startedAt, networkRequests: 0,
    passed: checks.length, snapshotsUnchanged: true, implementation: before, checks }, null, 2) + '\n');
  await writeFile(join(directory, 'report.md'), [
    '# Rust and TypeScript behavior parity', '', `${checks.length} checks passed. No network or paid API requests.`,
    'Compared exact chunks, shortlist order/scores, prepared Jev requests and byte-budget omissions against TypeScript. CLI checks compare exact successful JSON and matching acceptance/rejection for invalid input.',
    'Coverage includes deterministic stemmer corpus, word forms, camel/snake identifiers, ignored/binary/invalid UTF-8/oversize files, Unicode paths and IDs, BOM/CRLF/CR lines, chunk boundaries and generic JSON input.',
    'The prepared-request checks use a local fake response. Live Jev output is not measured or proven deterministic.', '',
    '| Check | Kind | Result |', '|---|---|---|', ...checks.map(check => `| ${check.id} | ${check.kind} | Pass |`), '',
  ].join('\n'));
  console.log(`${checks.length} parity checks passed (${words.length} stemmer inputs).`);
  console.log(`Report: ${join(directory, 'report.md')}`);
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
