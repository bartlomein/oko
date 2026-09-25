#!/usr/bin/env python3
"""Replay real agent questions against one Oko build and score the name layer.

The corpus is a JSONL of tool calls agents made to Oko (question, intent, deep,
directory, repo). Each question is sent to the build's `search` tool on a
pinned checkout of its repository and the packet is scored on what Phase 1
changes: does a named identifier come back as a definition, does the top
excerpt start at a declaration or mid-body, how often is the answer "No
relevant code found", how many bytes. The first call per repository runs on a
fresh cache, so it also records cold preparation time and the cache size.

  corpus.py --corpus plans/corpus/calls.jsonl --binary target/release/oko --label main
  corpus.py ... --jev           # rank with Jev: paid calls, ~$0.0013 each

Repositories are resolved through --repos (name=path, repeated) or the
defaults below; rows whose repository is not resolvable are skipped.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import replay  # noqa: E402

REFERENCE = Path('/Users/bart/dev/oko/benchmarks/results/sense-bench/sense-benchmark/_reference')
DEFAULT_REPOS = {
    **{name: REFERENCE / name for name in ('axum', 'discourse', 'flask', 'gin', 'javalin', 'nextjs')},
    **{name: Path('/Users/bart/dev') / name for name in ('astro', 'httpx', 'ripgrep')},
}
NRCF = 'No relevant code found'
# Identifier-shaped words: snake_case, camelCase, dotted or :: qualified, or a
# single Capitalized word (kept only under the rules in `identifiers`).
IDENTIFIER = re.compile(r'(?<![\w-])(?:[A-Za-z_][A-Za-z0-9_]*(?:(?:\.|::|#)[A-Za-z_][A-Za-z0-9_]*)+'
                        r'|[a-z0-9]+_[a-z0-9_]+|[a-z]+[A-Z][A-Za-z0-9]*|[A-Z][a-z0-9]+[A-Z][A-Za-z0-9]*'
                        r'|[A-Z][A-Za-z0-9]*[a-z][A-Za-z0-9]*)(?![\w-])')
SINGLE_CAPITALIZED = re.compile(r'^[A-Z][a-z0-9]+$')
STOP = {'ActiveRecord', 'GitHub', 'JavaScript', 'TypeScript', 'HTTP', 'API', 'URL', 'JSON', 'HTML', 'CSS', 'README',
        'Next', 'Flask', 'Django', 'Rails', 'Ruby', 'Python', 'Rust', 'Go', 'Java', 'Kotlin', 'Node', 'React', 'Astro',
        'Discourse', 'Javalin', 'Gin', 'Axum', 'HTTPX', 'Ripgrep', 'WSGI', 'ASGI', 'SSR', 'SSG', 'DOM', 'CLI', 'MCP',
        'SQL', 'ORM', 'JWT', 'CSRF', 'XSS', 'CORS', 'TLS', 'SSL', 'UTF', 'ASCII', 'PR', 'ID', 'OK', 'TODO', 'FIXME'}
# Sentence words agents capitalize: never symbols on their own.
PROSE = {'Trace', 'Locate', 'Return', 'Find', 'Show', 'Explain', 'List', 'How', 'What', 'Where', 'Which', 'Who',
         'Why', 'When', 'Describe', 'Identify', 'Compare', 'Check', 'Look', 'Get', 'Give', 'Include', 'Read',
         'Search', 'Follow', 'Map', 'Walk', 'Note', 'Also', 'Then', 'Provide', 'Focus', 'Start', 'Determine',
         'The', 'This', 'That', 'These', 'Those', 'For', 'From', 'With', 'Without', 'And', 'Or', 'But', 'In', 'On',
         'At', 'To', 'Of', 'If', 'Is', 'Are', 'Does', 'Do', 'Can', 'Should', 'Must', 'Will', 'Need', 'Please',
         'Content', 'Encoding', 'Type', 'Types', 'Error', 'Errors', 'Value', 'Values', 'Name', 'Names', 'File',
         'Files', 'Path', 'Paths', 'Code', 'Test', 'Tests', 'Data', 'Method', 'Methods', 'Function', 'Functions',
         'Class', 'Classes', 'Module', 'Modules', 'Config', 'Default', 'Defaults', 'Main', 'Base', 'Core', 'Server',
         'Client', 'Request', 'Response', 'Route', 'Routes', 'Router', 'Handler', 'Handlers', 'Model', 'Models',
         'View', 'Views', 'Controller', 'Controllers', 'Helper', 'Helpers', 'Service', 'Services', 'Component',
         'Components', 'Middleware', 'Plugin', 'Plugins', 'Image', 'Images', 'Page', 'Pages', 'Session', 'User',
         'Users', 'Post', 'Posts', 'Topic', 'Topics', 'Group', 'Groups', 'Category', 'Categories', 'Site', 'Theme',
         'Themes', 'Upload', 'Uploads', 'Email', 'Emails', 'Token', 'Tokens', 'Key', 'Keys', 'Cache', 'Store',
         'Context', 'Engine', 'Build', 'Dev', 'Static', 'Dynamic', 'Remote', 'Local', 'Public', 'Private'}
# A code noun right after a Capitalized word marks it as a symbol: "Upload model", "Captures trait".
CODE_NOUN = re.compile(r'^(?:class|trait|struct|enum|interface|module|model|type|component|hook|middleware|handler|'
                       r'service|controller|helper|mixin|decorator|macro|function|method|object|struct|impl|'
                       r'subclass|superclass|constructor|ActiveRecord|record|error|exception)\b', re.I)
KEYWORDS = r'(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|static\s+|export\s+|default\s+|abstract\s+|private\s+|public\s+|protected\s+|final\s+|unsafe\s+|const\s+)*'
DECLARATION_KINDS = r'(?:def|fn|func|function\*?|class|struct|enum|interface|trait|impl(?:<[^>]*>)?|module|mod|type|object|record|macro_rules!|@interface|protocol|extension)'


def identifiers(question):
    found = []
    for match in IDENTIFIER.finditer(question):
        word = match.group(0)
        if word in STOP or word.lower() in {'null', 'true', 'false'}:
            continue
        if SINGLE_CAPITALIZED.match(word):
            after = question[match.end():].lstrip()
            before = question[:match.start()].rstrip()
            marked = (CODE_NOUN.match(after)
                      or re.search(r'(?:class|trait|struct|module|model|interface|enum|type|struct|impl|`)\s*$', before, re.I)
                      or len(re.findall(rf'(?<![\w-]){re.escape(word)}(?![\w-])', question)) >= 2)
            sentence_start = before == '' or before[-1] in '.:;!?'
            if not marked and (word in PROSE or sentence_start):
                continue
        if word not in found:
            found.append(word)
    return found


FILE_EXTENSIONS = {'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'py', 'rb', 'rs', 'go', 'java', 'kt', 'cs', 'php', 'swift', 'scala'}


def leaf(identifier):
    parts = re.split(r'\.|::|#', identifier)
    # `Javalin.java` names the class Javalin, not a member called java.
    if len(parts) >= 2 and parts[-1] in FILE_EXTENSIONS:
        return parts[-2]
    return parts[-1]


def declares(line, name):
    """Does this source line look like a declaration of `name`?"""
    escaped = re.escape(name)
    patterns = (
        rf'^\s*{KEYWORDS}{DECLARATION_KINDS}\s+(?:[A-Za-z_][A-Za-z0-9_]*\s+)?(?:\([^)]*\)\s*)?{escaped}\b',  # def x / func (r) x / impl X
        rf'^\s*{KEYWORDS}(?:const|let|var|val|lazy val)\s+{escaped}\s*[:=]',                                # const x = / val x:
        rf'^\s*{KEYWORDS}{escaped}\s*(?:<[^>]*>)?\s*\([^)]*\)?\s*(?:[:{{]|=>|$)',                          # method x(...) {
        rf'^\s*{KEYWORDS}(?:[A-Za-z_][A-Za-z0-9_<>\[\],.\s]*\s+)?{escaped}\s*\(',                            # Type name( / func name(
        rf'^\s*{escaped}\s*[:=]\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_][A-Za-z0-9_]*)\s*=>',                 # x = (...) =>
        rf'^\s*(?:@\w+\s+)*{KEYWORDS}(?:[A-Za-z_][A-Za-z0-9_<>\[\],.]*\s+)+{escaped}\s*[;=({{]',              # Java field/method
    )
    return any(re.search(pattern, line) for pattern in patterns)


def declaration_hits(packet, names):
    """Names from the question that a shown excerpt declares (any line of any result)."""
    hits = set()
    excerpts = [e for kind in ('results', 'related') for e in packet.get(kind) or []]
    pins = (packet.get('floor') or {}).get('pins') or []
    for name in names:
        wanted = leaf(name)
        for excerpt in excerpts:
            symbol = (excerpt.get('symbol') or {}).get('name')
            if symbol == wanted or any(declares(line, wanted) for line in (excerpt.get('text') or '').splitlines()):
                hits.add(name)
                break
        else:
            # The name was pinned and a shown excerpt lies inside its body: a
            # member of the named class answers a question about that class.
            for pin in pins:
                if pin.get('name') == wanted and any(
                        e.get('path') == pin.get('path') and pin.get('startLine', 0) <= (e.get('startLine') or 0)
                        and (e.get('endLine') or 0) <= pin.get('endLine', 0) for e in excerpts):
                    hits.add(name)
                    break
    return hits


def first_line_kind(excerpt):
    """'declaration' when the excerpt starts at a declaration, 'mid' when it starts inside a body."""
    lines = [line for line in (excerpt.get('text') or '').splitlines() if line.strip()]
    if not lines:
        return 'empty'
    head = lines[0]
    symbol = (excerpt.get('symbol') or {})
    if symbol.get('line') == excerpt.get('startLine'):
        return 'declaration'
    for line in lines[:3]:  # allow a decorator/comment/attribute header
        stripped = line.strip()
        if stripped.startswith(('@', '#[', '//', '/*', '*', '#', '"""', "'''", '///')):
            continue
        if re.match(rf'^\s*{KEYWORDS}(?:{DECLARATION_KINDS}|const|let|var|val)\b', line) or re.match(r'^\S', line):
            return 'declaration'
        break
    return 'mid' if head[:1].isspace() else 'declaration'


def answer_body(text):
    """The answer after its coverage line and any notes."""
    lines = text.split('\n')
    while lines and (lines[0].startswith('Index: ') or lines[0] == '' or lines[0].startswith('`') and (' defined in ' in lines[0] or ' is used by ' in lines[0])
                     or lines[0].startswith('Paths are relative')):
        lines.pop(0)
    return '\n'.join(lines)


def first_label(text):
    text = answer_body(text)
    match = re.match(r'^\S+ \(([^)]*)\)', text)
    return match.group(1) if match else ('nrcf' if text.startswith(NRCF) else 'other')


def du(path):
    out = subprocess.run(['du', '-sk', str(path)], capture_output=True, text=True).stdout
    return int(out.split()[0]) * 1024 if out else None


def run_repo(repo, workspace, rows, binary, live, timeout, progress, extra_env=None):
    out = []
    cache_info = {}
    with tempfile.TemporaryDirectory(prefix='oko-corpus-cache-') as cache:
        client = replay.profiler().Client(Path(binary), workspace, Path(cache), timeout, live=live,
                                          api_key=replay.api_key() if live else None,
                                          model=replay.JEV_MODEL if live else None, extra_env=extra_env)
        try:
            client.initialize()
            for index, call in enumerate(rows):
                arguments = {'question': call['question']}
                for key in ('intent', 'deep', 'directory'):
                    if call.get(key) is not None:
                        arguments[key] = call[key]
                row = {'repo': repo, 'source': call['source'], 'session': call['session'], 'idx': call['idx'],
                       'question': call['question'], 'arguments': arguments}
                names = identifiers(call['question'])
                row['identifiers'] = names
                started = time.monotonic()
                try:
                    response = client.request('tools/call', {'name': 'search', 'arguments': arguments})
                    packet = replay.packet_of(client, response)
                    text = ''.join(b.get('text', '') for b in response.get('content', []))
                    retrieval = packet.get('retrieval') or {}
                    timings = packet.get('timings') or {}
                    results = packet.get('results') or []
                    hits = declaration_hits(packet, names)
                    row.update(
                        nrcf=not results,
                        # Listing answers (callers, tests) have no excerpts to score.
                        answerKind=('callers' if '\nCallers of ' in '\n' + answer_body(text) else
                                    'tests' if answer_body(text).startswith('Tests for ') else 'ranked'),
                        firstLabel=first_label(text),
                        firstLineKind=first_line_kind(results[0]) if results else None,
                        firstComplete=bool(results[0].get('definitionComplete')) if results else None,
                        results=len(results), related=len(packet.get('related') or []),
                        identifierHits=sorted(hits),
                        identifierHitAll=bool(names) and len(hits) == len(names),
                        identifierHitAny=bool(hits),
                        shown=[[e['path'], e['startLine'], e['endLine'], (e.get('symbol') or {}).get('name'),
                                bool(e.get('definitionComplete'))] for e in results],
                        textBytes=len(text), responseBytes=packet.get('responseBytes'),
                        # The excerpts themselves, so the scorer can be re-run offline.
                        excerpts=[{'path': e['path'], 'startLine': e['startLine'], 'endLine': e['endLine'],
                                   'symbol': (e.get('symbol') or {}).get('name'),
                                   'symbolLine': (e.get('symbol') or {}).get('line'),
                                   'text': e.get('text')}
                                  for kind in ('results', 'related') for e in packet.get(kind) or []],
                        okoMs=timings.get('totalMs'), cacheMs=(timings.get('cache') or {}).get('totalMs'),
                        firstInProcess=index == 0,
                        jevMs=retrieval.get('rerankMs'), jevCalls=len(retrieval.get('jevCalls') or []),
                        jevInputTokens=sum((c.get('usage') or {}).get('inputTokens') or 0
                                           for c in retrieval.get('jevCalls') or []),
                        ranking=packet.get('ranking'))
                    if index == 0:
                        cache_info = {'repo': repo, 'coldPrepareMs': (timings.get('cache') or {}).get('totalMs'),
                                      'readFiles': (timings.get('cache') or {}).get('readFiles'),
                                      'rebuiltFiles': (timings.get('cache') or {}).get('rebuiltFiles')}
                except Exception as error:  # A failed call is a result, not a reason to stop.
                    row['error'] = f'{type(error).__name__}: {error}'[:300]
                row['wallMs'] = round(1000 * (time.monotonic() - started))
                out.append(row)
                progress(row)
        finally:
            client.close()
        cache_info['cacheBytes'] = du(cache)
    return out, cache_info


def summarize(rows):
    scored = [r for r in rows if 'error' not in r]
    named = [r for r in scored if r['identifiers']]
    with_results = [r for r in scored if r.get('results')]
    def pct(n, d):
        return round(100 * n / d, 1) if d else None
    def med(key, subset=None):
        values = sorted(r[key] for r in (subset or scored) if r.get(key) is not None)
        return values[len(values) // 2] if values else None
    def p90(key):
        values = sorted(r[key] for r in scored if r.get(key) is not None)
        return values[int(len(values) * 0.9)] if values else None
    return {
        'calls': len(rows), 'scored': len(scored), 'errors': len(rows) - len(scored),
        'nrcfPct': pct(sum(r['nrcf'] for r in scored), len(scored)),
        'nrcfWithIdentifier': sum(r['nrcf'] for r in named),
        'namedCalls': len(named),
        'identifierHitAnyPct': pct(sum(r['identifierHitAny'] for r in named), len(named)),
        'identifierHitAllPct': pct(sum(r['identifierHitAll'] for r in named), len(named)),
        'firstCompletePct': pct(sum(bool(r['firstComplete']) for r in with_results), len(with_results)),
        'firstMidFunctionPct': pct(sum(r['firstLineKind'] == 'mid' for r in with_results), len(with_results)),
        'firstLabels': dict(sorted(defaultdict(int, {
            label: sum(1 for r in scored if r['firstLabel'] == label) for label in {r['firstLabel'] for r in scored}
        }).items(), key=lambda item: -item[1])),
        'textBytesMedian': med('textBytes'), 'textBytesP90': p90('textBytes'),
        'okoMsMedian': med('okoMs', [r for r in scored if not r.get('firstInProcess')]),
        'okoMsP90': p90('okoMs'),
        'jevMsMedian': med('jevMs'), 'jevCallsTotal': sum(r.get('jevCalls') or 0 for r in scored),
        'jevInputTokens': sum(r.get('jevInputTokens') or 0 for r in scored),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--corpus', required=True)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--label', required=True)
    parser.add_argument('--out', default=str(replay.PROJECT / 'benchmarks/results/corpus'))
    parser.add_argument('--repos', action='append', default=[], help='name=path, repeated')
    parser.add_argument('--only', help='Comma-separated repo names')
    parser.add_argument('--limit', type=int, default=0, help='Calls per repo')
    parser.add_argument('--jev', action='store_true')
    parser.add_argument('--env', action='append', default=[], help='KEY=VALUE passed to the Oko process (experiment knobs)')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    repos = dict(DEFAULT_REPOS)
    for item in args.repos:
        name, _, path = item.partition('=')
        repos[name] = Path(path)
    calls = [json.loads(line) for line in Path(args.corpus).read_text().splitlines() if line.strip()]
    by_repo = defaultdict(list)
    for call in calls:
        if call['repo'] in repos and repos[call['repo']].exists():
            if not args.only or call['repo'] in args.only.split(','):
                by_repo[call['repo']].append(call)
    skipped = len(calls) - sum(len(v) for v in by_repo.values())
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime('%Y%m%d-%H%M%S')
    out = out_dir / f'{args.label}-{"jev" if args.jev else "nojev"}-{stamp}.jsonl'
    rows, caches = [], []
    with out.open('w') as handle:
        for repo, selected in sorted(by_repo.items()):
            if args.limit:
                selected = selected[:args.limit]
            print(f'== {repo}: {len(selected)} calls', flush=True)
            def progress(row):
                handle.write(json.dumps(row) + '\n')
                handle.flush()
                mark = row.get('error') or f"{row['firstLabel']} hits={len(row['identifierHits'])}/{len(row['identifiers'])} {row['textBytes']}B {row['okoMs']}ms"
                print(f"  {row['idx']:>3} {mark}  {row['question'][:70]}", flush=True)
            repo_rows, cache = run_repo(repo, repos[repo], selected, args.binary, args.jev, args.timeout, progress,
                                        extra_env=dict(item.split('=', 1) for item in args.env))
            rows.extend(repo_rows)
            caches.append(cache)
    summary = {'label': args.label, 'jev': args.jev, 'binary': args.binary, 'skippedNoRepo': skipped,
               **summarize(rows), 'perRepo': {repo: summarize([r for r in rows if r['repo'] == repo])
                                              for repo in sorted(by_repo)}, 'caches': caches}
    out.with_suffix('.summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({k: v for k, v in summary.items() if k not in ('perRepo',)}, indent=2))
    print(out)


if __name__ == '__main__':
    main()
