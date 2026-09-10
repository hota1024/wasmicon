#!/usr/bin/env python3
"""Wasmicon ジェネレータの種。
`wasm-tools component wit --json wit/` の出力から、abi-spec.md §4 の規則で
Core Wasm の import シグネチャを導出して表示する。
abi-spec.md §7 の表と一致することが検証済み (2026-09-10)。

使い方:
    wasm-tools component wit --json wit > /tmp/hal.json
    python3 tools/wit2sig.py /tmp/hal.json

sig 表記: params:results, i=i32 I=i64 f=f32 F=f64
"""
import json, sys

d = json.load(open(sys.argv[1]))
T = d['types']

def kind(t):
    return T[t]['kind']

SCALAR = {'bool': 'i', 'u8': 'i', 'u16': 'i', 'u32': 'i', 's32': 'i',
          'u64': 'I', 's64': 'I', 'f32': 'f', 'f64': 'F', 'string': 'ii'}

def flat(t):
    """引数位置の lowering (abi-spec §4.1, §4.2)"""
    if isinstance(t, str):
        return SCALAR[t]
    k = kind(t)
    if k == 'resource':
        return 'i'
    key = next(iter(k))
    if key in ('enum', 'flags', 'handle', 'own', 'borrow'):
        return 'i'
    if key == 'list':
        assert k['list'] == 'u8', 'list<u8> 以外は未対応'
        return 'ii'
    if key == 'type':
        return flat(k['type'])
    raise SystemExit(f'未対応の型: {k}')

def outp(t):
    """result<T,_> の T を out ポインタに落とす (abi-spec §4.4)"""
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
