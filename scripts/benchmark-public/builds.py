"""Build immutable Git snapshots without changing the user's checkout."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare_builds(project,state,baseline_ref,current_ref):
    state.mkdir(parents=True,exist_ok=True)
    builds={}
    for label,ref in [('previous',baseline_ref),('current',current_ref)]:
        commit=subprocess.check_output(['git','rev-parse',ref+'^{commit}'],cwd=project,text=True).strip()
        dest=state/'builds'/commit;dest.mkdir(parents=True,exist_ok=True)
        binary=dest/'oko'
        record=dest/'build.json'
        rustc=subprocess.check_output(['rustc','--version'],text=True).strip()
        if record.exists():
            metadata=json.loads(record.read_text())
            if (metadata['sha256']!=digest(binary) or metadata['rustc']!=rustc
                    or metadata.get('targetIsolation')!='per-commit-v1'
                    or metadata.get('commit')!=commit or metadata.get('path')!=str(binary.resolve())):
                raise RuntimeError('Saved build changed; use a new build directory after inspection')
        else:
            print(f'Building {label}: {commit}',flush=True)
            with tempfile.TemporaryDirectory(prefix='oko-build-') as tmp:
                source=Path(tmp)
                archive=source/'source.tar'
                subprocess.run(['git','archive','--output',str(archive),commit],cwd=project,check=True)
                with tarfile.open(archive) as tar:tar.extractall(source,filter='data')
                target=project/'target/benchmark-builds'/commit
                env=dict(os.environ,CARGO_TARGET_DIR=str(target))
                with (dest/'build.log').open('w') as log:
                    subprocess.run(['cargo','build','--release','--locked','--offline','--bin','oko'],cwd=source,env=env,stdout=log,stderr=log,check=True)
                shutil.copy2(target/'release/oko',binary)
                # The new Rust contract loads the real interpolation module with
                # its real memchr dependency from this reproducible Cargo build.
                deps=list((target/'release/deps').glob('libmemchr-*.rlib'))
                if not deps:raise RuntimeError('Built memchr dependency missing')
                shutil.copy2(max(deps,key=lambda p:p.stat().st_mtime),state/'libmemchr.rlib')
            metadata=dict(path=str(binary.resolve()),commit=commit,sha256=digest(binary),rustc=rustc,targetIsolation='per-commit-v1')
            record.write_text(json.dumps(metadata,indent=2)+'\n')
        builds[label]=metadata
    if builds['previous']['commit']==builds['current']['commit']:
        raise ValueError('Previous and current refs resolve to the same commit')
    if builds['previous']['sha256']==builds['current']['sha256']:
        raise ValueError('Previous and current executables are identical; inspect build provenance')
    if not (state/'libmemchr.rlib').is_file():raise RuntimeError('Validator dependency missing')
    return builds


def prepare_working_build(project, state):
    """Freeze the exact working source for a new-client comparison, without committing it."""
    paths = sorted((project / 'src').rglob('*')) + [project / name for name in
            ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')]
    paths = [p for p in paths if p.is_file()]
    def fingerprint():
        return hashlib.sha256(''.join(str(p.relative_to(project)) + ':' + digest(p) for p in paths).encode()).hexdigest()
    source_hash = fingerprint()
    dest = state / 'builds' / ('working-' + source_hash)
    dest.mkdir(parents=True, exist_ok=True)
    record = dest / 'build.json'
    if record.exists():
        metadata = json.loads(record.read_text())
        if metadata['sha256'] != digest(Path(metadata['path'])):
            raise RuntimeError('Frozen working build changed')
        return metadata
    archive = dest / 'source.tar'
    with tarfile.open(archive, 'w') as tar:
        for path in paths:
            tar.add(path, arcname=str(path.relative_to(project)), recursive=False)
    # Reuse the checkout's release build cache; source hashes before and after
    # reject a concurrent edit instead of claiming it belongs to this snapshot.
    print('Building frozen working source: ' + source_hash[:12], flush=True)
    with (dest / 'build.log').open('w') as log:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--offline', '--bin', 'oko'],
                       cwd=project, stdout=log, stderr=log, check=True)
    if fingerprint() != source_hash:
        raise RuntimeError('Source changed during build; prepare again')
    binary = dest / 'oko'
    shutil.copy2(project / 'target/release/oko', binary)
    deps = list((project / 'target/release/deps').glob('libmemchr-*.rlib'))
    if not deps:
        raise RuntimeError('Built memchr dependency missing')
    shutil.copy2(max(deps, key=lambda p: p.stat().st_mtime), state / 'libmemchr.rlib')
    metadata = dict(path=str(binary.resolve()), sha256=digest(binary),
                    commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=project, text=True).strip(),
                    source='working-tree', sourceSha256=source_hash, sourceArchive=str(archive),
                    sourceArchiveSha256=digest(archive), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip())
    record.write_text(json.dumps(metadata, indent=2) + '\n')
    return metadata
