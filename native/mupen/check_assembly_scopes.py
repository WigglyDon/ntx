#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify actual assembly-scope sites and macros; never execute guest code."""
from pathlib import Path
import re
import resource
import subprocess

ROOT = Path(__file__).resolve().parents[2]
REL = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    source = (ROOT / '.local/source/mupen64plus-next' / REL).read_text()
    definitions = [line for line in source.splitlines() if line.startswith('#define NTX_ASM_EVENT(')]
    assert definitions == [
        '#define NTX_ASM_EVENT(phase, entering) retro_ntx_compile_event(42u + 2u * (phase) + (entering))',
        '#define NTX_ASM_EVENT(phase, entering) ((void)0)',
    ]
    sites = re.findall(r'^  NTX_ASM_EVENT\([0-3], [01]\);$', source, re.M)
    assert sites == [
        '  NTX_ASM_EVENT(0, 1);', '  NTX_ASM_EVENT(0, 0);',
        '  NTX_ASM_EVENT(1, 1);', '  NTX_ASM_EVENT(1, 0);',
        '  NTX_ASM_EVENT(2, 1);', '  NTX_ASM_EVENT(2, 0);',
        '  NTX_ASM_EVENT(3, 1);', '  NTX_ASM_EVENT(3, 0);',
    ]
    for anchor in [
        '  /* Pass 8 - Assembly */\n' + sites[0] + '\n  linkcount=0;stubcount=0;',
        sites[1] + '\n' + sites[2] + '\n  for(i=0;i<slen;i++)',
        sites[3] + '\n' + sites[4] + '\n  //assert(itype[i-2]==UJUMP',
        sites[5] + '\n' + sites[6] + '\n  // TODO: delay slot stubs?',
        sites[7] + '\n  NTX_PHASE_EVENT(7, 0);',
    ]:
        assert source.count(anchor) == 1, anchor
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    for mode, definition in [('active', definitions[0]), ('disabled', definitions[1])]:
        prefix = '#include <assert.h>\nstatic unsigned count;\n'
        if mode == 'active':
            prefix += 'static unsigned seen[8];\nstatic void retro_ntx_compile_event(unsigned value) { assert(count<8);seen[count++]=value; }\n'
        fixture = prefix + definition + '\nint main(void) {\n' + '\n'.join(sites) + '\n'
        if mode == 'active':
            fixture += 'const unsigned expected[8]={43,42,45,44,47,46,49,48};assert(count==8);for(unsigned i=0;i<8;i++)assert(seen[i]==expected[i]);\n'
        else:
            fixture += 'assert(count==0);\n'
        fixture += 'return 0;\n}\n'
        path = out / ('assembly-scopes-' + mode + '.c')
        path.write_text(fixture)
        for suffix, flags in [('normal', []), ('trap', ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error'])]:
            exe = out / ('assembly-scopes-' + mode + '-' + suffix)
            subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *flags, str(path), '-o', str(exe)], check=True, timeout=20)
            subprocess.run([str(exe)], check=True, timeout=10)
    print('Actual assembly scope sites/macros: ordered setup/emission/tail/stubs, active/disabled, normal/UBSan trap; no compiler execution or performance claim')


if __name__ == '__main__':
    main()
