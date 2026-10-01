#!/usr/bin/env python3
"""Check the prompt hook's gate: code prompts get Oko's answer, other prompts get nothing.

Every prompt goes through the running server exactly as the hook sends it
(`search` with `prefetch`), each in a session of its own, against the agent
suite's repositories (astro, httpx, ripgrep).

Free by default: a local stand-in answers the ranker's requests with scores of
0.02, so nothing is injected and the check counts which prompts would reach
Jev at all. `--execute` uses the real Jev and shows what would be injected.

Targets (plans/prefetch.md): no non-code prompt injects more than 1 KB, at most
a quarter of non-code prompts reach Jev, and at least 70% of code prompts are
answered.
"""
import argparse
import json
from pathlib import Path
import sys
import tarfile
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import replay  # noqa: E402
import verdicts  # noqa: E402

# Replies, chores, commands and questions about anything but this code.
NON_CODE = [
    'yes', 'ok go ahead', 'thanks, that works', 'continue', 'try again please',
    'looks good, ship it', 'no, undo that', 'stop', 'lgtm', 'perfect thank you so much',
    'commit this and push', 'open a pull request for this branch', 'rename the branch to feat/cleanup',
    'squash the last three commits', 'what changed since yesterday?', 'run the tests again',
    'run the linter and fix the formatting', 'bump the version and tag a release',
    'delete the temporary files you created', 'show me the git log for today',
    'write a haiku about autumn', 'what is the capital of Portugal?',
    'summarize this conversation in three bullet points', 'draft an email to the team about the release',
    'translate the release notes into Polish', 'what time zone is Tokyo in?',
    'explain the difference between a mutex and a semaphore in general terms',
    'how are you doing today?', 'give me a pep talk before the demo',
    'plan my week: gym on Monday, dentist on Thursday',
    'can you make the answer shorter?', 'use bullet points instead', 'say that again in plain English',
    'why did you do that?', 'that is not what I asked for, start over', 'remember to keep answers brief',
    'what model are you?', 'how much did this session cost?', 'please be more careful next time',
    'ok now do the same for the other one',
]

# User-style questions about the suite's repositories, and the suite's own task sentences.
CODE = {
    'httpx': [
        'where does httpx follow redirects and decide when to stop?',
        'how does the client apply timeouts to a request?',
        'why does `Client.send` close the response on error?',
        'explain how DigestAuth builds the authorization header',
        'where are cookies merged into an outgoing request?',
        'how does httpx pick a transport for a proxy URL?',
        'find where the request body is streamed for async clients',
        'who calls build_request?',
        'what does raise_for_status do for 3xx responses?',
        'where is the default User-Agent header set?',
    ],
    'ripgrep': [
        'where does ripgrep decide to skip hidden files?',
        'how are .gitignore rules loaded and applied?',
        'explain how the searcher handles binary files',
        'where is the --replace text expanded for each match?',
        'how does the printer compute line numbers for context lines?',
        'find where glob patterns from --glob are compiled',
        'why does ripgrep stop reading a file after the first match with -l?',
        'where is the thread count for parallel search chosen?',
        'how does ripgrep detect the encoding of a file?',
        'tests for the standard printer summary output',
        'where are ripgrep config file arguments parsed?',
    ],
    'astro': [
        'where does astro validate remote image URLs?',
        'how are actions resolved from their names on the server?',
        'explain how i18n routing picks the locale for a request',
        'where is middleware sequenced for a page render?',
        'how does astro read the forwarded host header?',
        'find where content collections are loaded from disk',
        'why does the dev server reload when astro.config changes?',
        'where are redirects defined in the config applied at build time?',
        'how does the image service compute output dimensions?',
        'who calls getFirstForwardedValue?',
    ],
}


def cases(tasks):
    for repository, prompts in CODE.items():
        for prompt in prompts:
            yield repository, 'code', prompt
    for task, (repository, definition) in tasks.items():
        yield repository, 'code', definition['question']
    for index, prompt in enumerate(NON_CODE):
        # Chores are asked in every repository alike; spread them over the three.
        yield ('httpx', 'ripgrep', 'astro')[index % 3], 'non-code', prompt


def run(binary, live, timeout):
    tasks = replay.load_tasks()
    rows = []
    stand_in = None if live else verdicts.StandInJev()
    with tempfile.TemporaryDirectory(prefix='oko-gate-') as scratch:
        workspaces = {}
        for repository in ('httpx', 'ripgrep', 'astro'):
            workspace = Path(scratch) / repository
            workspace.mkdir()
            with tarfile.open(replay.BRANCH_STATE / repository / 'baseline.tar') as archive:
                archive.extractall(workspace, filter='data')
            workspaces[repository] = workspace
        by_repository = {}
        for index, (repository, kind, prompt) in enumerate(cases(tasks)):
            by_repository.setdefault(repository, []).append((index, kind, prompt))
        for repository, items in by_repository.items():
            cache = Path(scratch) / f'cache-{repository}'
            cache.mkdir()
            client = replay.profiler().Client(
                Path(binary), workspaces[repository], cache, timeout, live=True,
                api_key=replay.api_key() if live else 'dummy-stand-in', model=replay.JEV_MODEL,
                extra_env=None if live else verdicts.environment(stand_in, {}))
            try:
                client.initialize()
                for index, kind, prompt in items:
                    if stand_in:
                        stand_in.requests = []
                    response = client.request('tools/call', {'name': 'search', 'arguments': {
                        'question': prompt, 'prefetch': f'gate-{index}'}})
                    text = ''.join(b.get('text', '') for b in response.get('content', []))
                    hook = json.loads(text) if text.strip().startswith('{') else {}
                    context = (hook.get('hookSpecificOutput') or {}).get('additionalContext') or ''
                    metrics = client.last_metrics() or {}
                    decision = metrics.get('prefetch') or {}
                    jev = (len(stand_in.requests) if stand_in
                           else len((metrics.get('retrieval') or {}).get('jevCalls') or []))
                    rows.append({'repository': repository, 'kind': kind, 'prompt': prompt,
                                 'decision': decision.get('decision'), 'reason': decision.get('reason'),
                                 'jevRequests': jev, 'injectedChars': len(context),
                                 'totalMs': (metrics.get('timings') or {}).get('totalMs')})
                    print(f"{kind:8} {repository:7} {rows[-1]['decision'] or '?':7} "
                          f"{rows[-1]['reason'] or '':18} jev={jev} chars={len(context):5}  {prompt[:70]}",
                          flush=True)
            finally:
                client.close()
    if stand_in:
        stand_in.close()
    return rows


def summarize(rows, live):
    def share(part, whole):
        return f'{part}/{whole} ({100 * part / whole:.0f}%)' if whole else '0/0'
    code = [r for r in rows if r['kind'] == 'code']
    other = [r for r in rows if r['kind'] == 'non-code']
    lines = [
        f"Non-code prompts reaching Jev: {share(sum(r['jevRequests'] > 0 for r in other), len(other))} (target ≤ 25%)",
        f"Code prompts reaching Jev: {share(sum(r['jevRequests'] > 0 for r in code), len(code))}",
    ]
    if live:
        lines += [
            f"Non-code prompts injecting > 1 KB: {sum(r['injectedChars'] > 1000 for r in other)} (target 0)",
            f"Non-code prompts injecting anything: {sum(r['injectedChars'] > 0 for r in other)}",
            f"Code prompts answered: {share(sum(r['decision'] in ('inject', 'pointer') for r in code), len(code))} (target ≥ 70%)",
        ]
    else:
        lines.append('Free run: the stand-in rates everything 0.02, so nothing is injected; '
                     'add --execute to see what real Jev scores inject.')
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True, help='Oko build to check')
    parser.add_argument('--execute', action='store_true', help='Use the real Jev (paid)')
    parser.add_argument('--timeout', type=float, default=60)
    args = parser.parse_args(argv)
    rows = run(args.binary, args.execute, args.timeout)
    summary = summarize(rows, args.execute)
    replay.STATE.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix='gate-', dir=replay.STATE))
    (output / 'gate.json').write_text(json.dumps({'binary': args.binary, 'live': args.execute,
                                                   'rows': rows}, indent=2) + '\n')
    print('\n' + summary)
    print(output)
    return 0


if __name__ == '__main__':
    sys.exit(main())
