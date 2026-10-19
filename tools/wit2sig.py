#!/usr/bin/env python3
"""Reference derivation of the Core Wasm import signatures.

Reads the output of `wasm-tools component wit --json wit/` and derives the
signature of every import by the rules of docs/abi-spec.md §4. The result must
match the table of docs/abi-spec.md §7; tools/check-sigs.sh cross-checks it
against wasmicon-gen.

Usage:
    wasm-tools component wit --json wit > /tmp/hal.json
    python3 tools/wit2sig.py /tmp/hal.json

Notation: params:results, i=i32 I=i64 f=f32 F=f64
"""
import json, sys

d = json.load(open(sys.argv[1]))
T = d['types']

def kind(t):
    return T[t]['kind']

SCALAR = {'bool': 'i', 'u8': 'i', 'u16': 'i', 'u32': 'i', 's32': 'i',
          'u64': 'I', 's64': 'I', 'f32': 'f', 'f64': 'F', 'string': 'ii'}

def flat(t):
    """Lowers one parameter (docs/abi-spec.md §4.1, §4.2)."""
    if isinstance(t, str):
        return SCALAR[t]
    k = kind(t)
    if k == 'resource':
        return 'i'
    key = next(iter(k))
    if key in ('enum', 'flags', 'handle', 'own', 'borrow'):
        return 'i'
    if key == 'list':
        assert k['list'] == 'u8', 'only list<u8> is supported'
        return 'ii'
    if key == 'type':
        return flat(k['type'])
    raise SystemExit(f'unsupported type: {k}')

def outp(t):
    """Lowers the T of result<T,_> to an out-pointer (docs/abi-spec.md §4.4)."""
    if not isinstance(t, str) and 'list' in kind(t):
        return 'iii'  # buf, cap, len-out
    return 'i'

for iface in d['interfaces']:
    print(f"--- wasmicon:hal/{iface['name']}@0.1.0")
    for fname, f in iface['functions'].items():
        sig = ''.join(flat(p['type']) for p in f['params'])
        r = f.get('result')
        if r is None:
            ret = ''
        elif not isinstance(r, str) and 'result' in kind(r):
            ok = kind(r)['result']['ok']
            if ok is not None:
                sig += outp(ok)
            ret = 'i'
        else:
            ret = flat(r)
        print(f"  {fname:26s} {sig}:{ret}")
    for tname, t in iface['types'].items():
        if kind(t) == 'resource':
            print(f"  [resource-drop]{tname:12s} i:")
