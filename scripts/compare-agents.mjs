import { main } from './compare-codex.mjs';
main({ tools: 'oko,codex,opencode', fixture: 'benchmarks/telemetry-studio-expanded.json' })
  .catch(error => { console.error(error.message); process.exitCode = 1; });
