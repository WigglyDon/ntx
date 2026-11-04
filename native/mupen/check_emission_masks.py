#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Actual three emission loaders preserve the complete guest-width mask."""
from pathlib import Path
import resource
import re
import signal
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[2]
REL = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'
SIGNATURES = ['static void load_regs(', 'static void load_consts(',
              'static void load_all_consts(']


def function(source, signature):
    # These helpers contain commented-out opening braces. Preserve source bytes
    # for compilation/comparison while ignoring comments/string braces in nesting.
    masked = re.sub(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'',
                    lambda match: ' ' * len(match.group()), source)
    start = source.index(signature)
    brace = masked.index('{', start)
    depth, end = 1, brace + 1
    while depth:
        depth += (masked[end] == '{') - (masked[end] == '}')
        end += 1
    return source[start:end]


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    source = (ROOT / '.local/source/mupen64plus-next' / REL).read_text()
    with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + REL))
        original = archive.extractfile(member).read().decode()
    before = '--original' in sys.argv
    bodies = []
    for signature in SIGNATURES:
        old = function(original, signature)
        active = old if before else function(source, signature)
        assert old.count('int is32') == 1
        assert active == (old if before else old.replace('int is32', 'uint64_t is32', 1))
        bodies.append(active)
    fixture = r'''
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef unsigned int u_int;
#define HOST_REGS 29
#define EXCLUDE_REG 29
enum { ALU, UJUMP, RJUMP, CJUMP, SJUMP, FJUMP };
static struct {signed char regmap[HOST_REGS]; unsigned isconst;} regs[4];
static int constmap[4][HOST_REGS],bt[4],itype[4],slen;
static uint64_t unneeded_reg[4],unneeded_reg_upper[4];
static signed char entry[HOST_REGS];
enum { ZERO, LOAD, SIGN, IMM };
static int seen[4],calls;
static void record(int op,int a,int b,int c) {assert(calls++==0);seen[0]=op;seen[1]=a;seen[2]=b;seen[3]=c;}
static void emit_zeroreg(int hr) {record(ZERO,hr,0,0);}
static void emit_loadreg(int reg,int hr) {record(LOAD,reg,hr,0);}
static void emit_sarimm(int lo,int bits,int hi) {record(SIGN,lo,bits,hi);}
static void emit_movimm(int value,int hr) {record(IMM,value,hr,0);}
''' + function(source, 'static signed char get_reg(') + '\n'
    fixture += function(source, 'static int get_final_value(') + '\n' + '\n'.join(bodies)
    fixture += r'''
static void initialize(int reg,int hi,int value) {
 calls=0;slen=1;bt[0]=0;itype[0]=ALU;
 for(int h=0;h<HOST_REGS;h++) {entry[h]=-1;regs[0].regmap[h]=-1;constmap[0][h]=0;}
 int lo=(hi+1)%HOST_REGS;
 regs[0].regmap[hi]=(signed char)(reg+64);regs[0].regmap[lo]=(signed char)reg;
 entry[lo]=(signed char)reg;regs[0].isconst=1u<<hi;constmap[0][hi]=value;
}
static void invoke(int which,int reg,int hi,uint64_t mask) {
 if(which==0) load_regs(entry,regs[0].regmap,mask,reg,-1);
 if(which==1) load_consts(entry,regs[0].regmap,mask,0);
 if(which==2) load_all_consts(regs[0].regmap,mask,1u<<hi,1u<<hi,0);
}
int main(int argc,char **argv) {
 if(argc>1) {
  // HI is guest register32, LO33. Their real flags live above an int's width.
  initialize(32,1,777);invoke(atoi(argv[1]),32,1,UINT64_C(1)<<32);return 0;
 }
 int values[]={0,1,-1,INT32_MIN,INT32_MAX,0x12345678};unsigned checked=0;
 for(int which=0;which<3;which++) for(int reg=1;reg<64;reg++)
 for(int hi=0;hi<HOST_REGS;hi++) for(unsigned n=0;n<sizeof(values)/sizeof(values[0]);n++)
 for(int sign=0;sign<2;sign++) for(int others=0;others<2;others++) {
  uint64_t bit=UINT64_C(1)<<reg;
  uint64_t mask=(others?UINT64_MAX^bit:0)|(sign?bit:0);
  initialize(reg,hi,values[n]);invoke(which,reg,hi,mask);
  assert(calls==1);
  int lo=(hi+1)%HOST_REGS;
  if(sign) {assert(seen[0]==SIGN && seen[1]==lo && seen[2]==31 && seen[3]==hi);}
  else if(which==0) {assert(seen[0]==LOAD && seen[1]==reg+64 && seen[2]==hi);}
  else if(values[n]==0) {assert(seen[0]==ZERO && seen[1]==hi);}
  else {assert(seen[0]==IMM && seen[1]==values[n] && seen[2]==hi);}
  checked++;
 }
 for(int reg=1;reg<64;reg++) for(int hi=0;hi<HOST_REGS;hi++) {
  int lo=(hi+1)%HOST_REGS,other=(hi+2)%HOST_REGS;
  initialize(reg,hi,777);regs[0].regmap[lo]=-1;entry[lo]=-1;
  invoke(0,reg,hi,UINT64_C(1)<<reg);
  assert(calls==1 && seen[0]==LOAD && seen[1]==reg+64 && seen[2]==hi);checked++;
  initialize(reg,hi,777);regs[0].regmap[other]=(signed char)reg;entry[other]=(signed char)reg;
  invoke(0,reg,hi,UINT64_C(1)<<reg);
  assert(calls==1 && seen[0]==SIGN && seen[1]==(lo<other?lo:other) && seen[2]==31 && seen[3]==hi);checked++;
 }
 // Retain the original admission conditions: dirty/constant flags and previous
 // constant-map continuity decide whether any load is emitted at all.
 volatile uint64_t mask33=UINT64_C(1)<<33;
 initialize(33,2,777);load_all_consts(regs[0].regmap,mask33,0,1u<<2,0);assert(calls==0);
 initialize(33,2,777);load_all_consts(regs[0].regmap,mask33,1u<<2,0,0);assert(calls==0);
 initialize(33,2,777);slen=2;regs[1]=regs[0];regs[0].isconst=1u<<2;
 for(int h=0;h<HOST_REGS;h++) entry[h]=regs[1].regmap[h];
 bt[1]=0;load_consts(entry,regs[1].regmap,mask33,1);assert(calls==0);
 bt[1]=1;load_consts(entry,regs[1].regmap,mask33,1);assert(calls==1 && seen[0]==SIGN);
 printf("Actual three emission loaders retain full64-bit mask and operation decisions in %u cases; no timing claim\n",checked);
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / ('emission-masks-original.c' if before else 'emission-masks.c')
    path.write_text(fixture)
    for suffix, flags in [('normal', []), ('trap', ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error'])]:
        if before and suffix == 'normal':
            continue
        exe = out / (path.stem + '-' + suffix)
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *flags,
                        str(path), '-o', str(exe)], check=True, timeout=25)
        if before:
            for which in range(3):
                result = subprocess.run([str(exe), str(which)], timeout=15)
                assert result.returncode == -signal.SIGILL, 'original mask narrowing must trap'
            print('All three actual original int-mask loaders trap under UBSan for guest register32')
        else:
            subprocess.run([str(exe)], check=True, timeout=30)


if __name__ == '__main__':
    main()
