#!/usr/bin/env python3
"""Dependency-free, executable checks against actual edited source modules."""
import importlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types


def check(task_id, work):
    work = Path(work).resolve()
    if task_id == 'astro-forwarded-empty':
        with tempfile.TemporaryDirectory() as temp:
            module=Path(temp)/'request.mts'
            module.write_bytes((work/'packages/internal-helpers/src/request.ts').read_bytes())
            script='import assert from "node:assert/strict"; import * as m from '+json.dumps(module.as_uri())+';\n'
            script+='''
                for (const value of ['', '  ', ',later', ' ,later', [], [''], null, undefined])
                    assert.equal(m.getFirstForwardedValue(value), undefined);
                for (const [value, expected] of [[' a ,b','a'], [['a','b'],'a'], ['0','0'], ['::1','::1']])
                    assert.equal(m.getFirstForwardedValue(value),expected);
                assert.equal(m.getValidatedIpFromHeader('127.0.0.1,::1'),'127.0.0.1');
                assert.equal(m.getValidatedIpFromHeader('<bad>'),undefined);
            '''
            subprocess.run(['node','--experimental-strip-types','--input-type=module','-e',script],check=True,timeout=30)
        return
    if task_id == 'httpx-reason-fallback':
        spec=importlib.util.spec_from_file_location('subject',work/'httpx/_status_codes.py')
        m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
        for value in (-1,0,199,299,599,999):assert m.codes.get_reason_phrase(value)=='Unknown Status'
        for value,phrase in [(200,'OK'),(404,'Not Found'),(m.codes.IM_A_TEAPOT,"I'm a teapot")]:
            assert m.codes.get_reason_phrase(value)==phrase
        assert m.codes.is_success(299) and not m.codes.is_success(300)
        assert m.codes.is_error(599) and not m.codes.is_error(600)
        return
    if task_id == 'ripgrep-capture-hyphen':
        library=Path(__file__).resolve().parents[2]/'benchmarks/results/public-branch/libmemchr.rlib'
        with tempfile.TemporaryDirectory() as temp:
            harness=Path(temp)/'check.rs';binary=Path(temp)/'check'
            source=work/'crates/matcher/src/interpolate.rs'
            harness.write_text('include!('+json.dumps(str(source))+');\n'+r'''
                #[test] fn benchmark_contract() {
                    for (input,expected) in [("$first-name", "MATCH"), ("${first-name}","MATCH"),
                        ("a${first-name}b","aMATCHb"), ("$$first-name","$first-name"),
                        ("$1","MATCH"), ("${1}","MATCH"), ("$first_name","MATCH"),
                        ("${first-name","${first-name"), ("$","$"), ("$unknown", "")] {
                        let mut out=vec![];
                        interpolate(input.as_bytes(), |i,out| { if i==1 {out.extend(b"MATCH")} },
                            |name| if name=="first-name" || name=="first_name" {Some(1)} else {None}, &mut out);
                        assert_eq!(out,expected.as_bytes(),"{}",input);
                    }
                }
            ''')
            subprocess.run(['rustc','--edition=2021','--test',str(harness),'--extern','memchr='+str(library),'-o',str(binary)],check=True,timeout=45)
            subprocess.run([str(binary)],check=True,timeout=15)
        return
    if task_id.startswith('astro-'):
        name, cases = {
            'astro-query-delimiter': ('removeQueryString', [('', ''), ('?q=a', ''), ('/docs?q=a?b', '/docs'), ('/docs?', '/docs'), ('/docs', '/docs'), ('/a%3Fb', '/a%3Fb')]),
            'astro-file-extension': ('removeFileExtension', [('file', 'file'), ('/a.b/file', '/a.b/file'), ('.env', '.env'), ('/a/.env', '/a/.env'), ('/a.b/file.txt', '/a.b/file'), ('archive.tar.gz', 'archive.tar'), ('.env.local', '.env'), ('', '')]),
        }[task_id]
        with tempfile.TemporaryDirectory() as temp:
            module = Path(temp) / 'path.mts'
            module.write_bytes((work / 'packages/internal-helpers/src/path.ts').read_bytes())
            script = 'import assert from "node:assert/strict"; import * as m from ' + json.dumps(module.as_uri()) + ';\n'
            script += f'for (const [input, expected] of {json.dumps(cases)}) assert.equal(m[{json.dumps(name)}](input), expected);'
            subprocess.run(['node', '--experimental-strip-types', '--input-type=module', '-e', script], check=True, timeout=30)
    elif task_id.startswith('httpx-'):
        # Load the real utility module and its real type module without importing
        # HTTPX's network transports or requiring third-party packages.
        package = types.ModuleType('httpx')
        package.__path__ = [str(work / 'httpx')]
        sys.modules['httpx'] = package
        m = importlib.import_module('httpx._utils')
        if task_id == 'httpx-empty-unquote':
            for value, expected in [('', ''), ('"', '"'), ('""', ''), ('"hello"', 'hello'), ('hello', 'hello'), ("'hi'", "'hi'"), ('"hi', '"hi')]:
                assert m.unquote(value) == expected, repr(value)
        elif task_id == 'httpx-closed-stream':
            import io
            b = io.BytesIO(b'abcdef'); b.seek(2)
            assert m.peek_filelike_length(b) == 6
            assert b.tell() == 2
            b.close()
            assert m.peek_filelike_length(b) is None
            with tempfile.TemporaryFile() as f:
                f.write(b'abcdef'); f.flush(); f.seek(3)
                assert m.peek_filelike_length(f) == 6
                assert f.tell() == 3
            assert m.peek_filelike_length(f) is None
            assert m.peek_filelike_length(object()) is None
            class NoRead(io.BytesIO):
                def read(self, *args):
                    raise AssertionError('must not read stream')
            assert m.peek_filelike_length(NoRead(b'abc')) == 3
        else:
            raise ValueError(task_id)
    elif task_id.startswith('ripgrep-'):
        source = work / 'crates/cli/src/human.rs'
        assertions = {
            'ripgrep-lowercase-size': '''
                for (s, expected) in [("2k", 2048), ("3m", 3<<20), ("4g", 4<<30), ("0k", 0), ("2K", 2048), ("23", 23)] {
                    assert_eq!(parse_human_readable_size(s).unwrap(), expected);
                }
                for s in ["", "1T", "1KB", " 1k", "1k ", "-1k", "18446744073709551615g", "18446744073709551616"] {
                    assert!(parse_human_readable_size(s).is_err(), "{}", s);
                }
            ''',
            'ripgrep-size-error-kind': '''
                for s in ["", "1T", "18446744073709551616", "18446744073709551615G"] {
                    let err = parse_human_readable_size(s).unwrap_err();
                    let text = err.to_string();
                    let io: std::io::Error = err.into();
                    assert_eq!(io.kind(), std::io::ErrorKind::InvalidInput);
                    assert_eq!(io.to_string(), text);
                    assert!(io.get_ref().unwrap().is::<ParseSizeError>());
                }
                assert_eq!(parse_human_readable_size("2K").unwrap(), 2048);
                assert!(parse_human_readable_size("2k").is_err());
            ''',
        }[task_id]
        with tempfile.TemporaryDirectory() as temp:
            harness = Path(temp) / 'check.rs'
            harness.write_text('include!(' + json.dumps(str(source)) + ');\n#[test] fn benchmark_contract() {' + assertions + '}\n')
            binary = Path(temp) / 'check'
            subprocess.run(['rustc', '--edition=2021', '--test', str(harness), '-o', str(binary)], check=True, timeout=45)
            subprocess.run([str(binary)], check=True, timeout=15)
    else:
        raise ValueError(task_id)


if __name__ == '__main__':
    check(sys.argv[1], sys.argv[2])
