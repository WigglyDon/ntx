#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Actual incoming-branch index vs all four original live-analysis searches."""
from pathlib import Path
import re
import resource
import subprocess
import tarfile
from check_register_membership import function

ROOT = Path(__file__).resolve().parents[2]
REL = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'
SOURCE = ROOT / '.local/source/mupen64plus-next' / REL


def original_source():
    with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + REL))
        return archive.extractfile(member).read().decode()


def fragments(original):
    result = []
    start = original.index('static void provisional_32bit(void)')
    first = original.index('      for(j=i-1;j>=0;j--)', start)
    end = original.index('      is32=temp_is32;', first)
    result.append(original[first:end])
    start = original.index('/* Pass 3 - Register allocation */')
    for _ in range(3):
        first = original.index('      for(j=i-1;j>=0;j--)', start)
        end = original.index('      if(temp_is32!=current.is32)', first)
        result.append(original[first:end])
        start = end + 1
    return result


def declared_body(body):
    for delta, old in enumerate(fragments(original_source())[1:]):
        assert body.count(old) == 1
        body = body.replace(old, f'      temp_is32=ntx_incoming_is32(i+{delta},i,temp_is32,0);\n')
    anchor = '  /* Pass 3 - Register allocation */\n'
    assert body.count(anchor) == 1
    return body.replace(anchor, anchor + '  ntx_index_incoming();\n')


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    source = SOURCE.read_text()
    original = original_source()
    refs = fragments(original)
    provisional = function(original, 'static void provisional_32bit(void)')
    provisional = provisional.replace('int i,j;', 'int i;').replace(refs[0], '      temp_is32=ntx_incoming_is32(i,i,temp_is32,1);\n')
    assert provisional == function(source, 'static void provisional_32bit(void)')
    # Destinations are immutable after the index is built; only values are live.
    remainder = source[source.index('  ntx_index_incoming();'):]
    assert not re.search(r'\bba\s*\[[^]]+\]\s*=(?!=)', remainder)
    assert source.count('  ntx_index_incoming();') == 1
    assert source.count('ntx_incoming_is32(') == 5  # helper plus four searches
    fixture = '''
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#define MAXBLOCK 4096
static uint32_t start,ba[MAXBLOCK];
static int slen;
static uint64_t p32[MAXBLOCK];
static struct {uint64_t is32;} branch_regs[MAXBLOCK];
'''
    for n, body in enumerate(refs):
        fixture += f'static uint64_t original{n}(int i,uint64_t seed) {{int j;uint64_t temp_is32=seed;\n' + body + 'return temp_is32;}\n'
    fixture += 'static int ntx_incoming_head[MAXBLOCK],ntx_incoming_next[MAXBLOCK];\n'
    fixture += function(source, 'static void ntx_index_incoming(void)') + '\n'
    fixture += function(source, 'static uint64_t ntx_incoming_is32(') + '\n'
    fixture += r'''
static uint32_t rng=0x7c1e584du;
static uint32_t random32(void) {rng^=rng<<13;rng^=rng>>17;rng^=rng<<5;return rng;}
static uint64_t random64(void) {return ((uint64_t)random32()<<32)|random32();}
int main(void) {
 unsigned sizes[]={4096,1,4095,2,3,31,32,33,127,1024,1};
 unsigned starts[]={0,0x80000000u,0xbfc00000u,0xffffff00u,0xffffffffu,0xfffffffeu};
 unsigned checked=0;
 for(unsigned n=0;n<sizeof(sizes)/sizeof(sizes[0]);n++) {
  slen=(int)sizes[n];
  for(unsigned address=0;address<6;address++) for(unsigned pattern=0;pattern<5;pattern++) {
   start=starts[address];
   for(int j=0;j<slen;j++) {
    unsigned t=random32()%(unsigned)slen;
    ba[j]=pattern==0?start+t*4u:pattern==1?start:pattern==2?UINT32_MAX:pattern==3?random32():start+(slen-1)*4u;
    p32[j]=random64();branch_regs[j].is32=random64();
   }
   ntx_index_incoming();
   for(int i=0;i<slen;i++) {
    // Mutate values after indexing: future p32 and past branch_regs must be read live.
    unsigned changed=random32()%(unsigned)slen;
    p32[changed]=random64();branch_regs[changed].is32=random64();
    uint64_t seed=random64();
    assert(original0(i,seed)==ntx_incoming_is32(i,i,seed,1));
    assert(original1(i,seed)==ntx_incoming_is32(i,i,seed,0));
    if(i+1<slen) assert(original2(i,seed)==ntx_incoming_is32(i+1,i,seed,0));
    if(i+2<slen) assert(original3(i,seed)==ntx_incoming_is32(i+2,i,seed,0));
    checked++;
   }
  }
 }
 printf("Actual incoming-branch index matches four retained searches for %u changing-value queries; no timing claim\n",checked);
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / 'incoming.c'
    path.write_text(fixture)
    for suffix, flags in [('normal', []), ('trap', ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error'])]:
        exe = out / ('incoming-' + suffix)
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *flags, str(path), '-o', str(exe)], check=True, timeout=25)
        subprocess.run([str(exe)], check=True, timeout=40)


if __name__ == '__main__':
    main()
