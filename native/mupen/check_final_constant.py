#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Actual emission constant reader: upper-half masks and retained scan rules."""
from pathlib import Path
import resource
import signal
import subprocess
import sys
import tarfile
from check_register_membership import function

ROOT = Path(__file__).resolve().parents[2]
REL = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'
SOURCE = ROOT / '.local/source/mupen64plus-next' / REL


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + REL))
        original = function(archive.extractfile(member).read().decode(), 'static int get_final_value(')
    before = '--original' in sys.argv
    active = original if before else function(SOURCE.read_text(), 'static int get_final_value(')
    old = 'return !((unneeded_reg_upper[i+1]>>reg)&1);'
    new = 'return !((unneeded_reg_upper[i+1]>>(reg&63))&1);'
    assert original.count(old) == 1
    assert active == (original if before else original.replace(old, new))
    fixture = r'''
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#define MAXBLOCK 4096
#define HOST_REGS 29
enum { ALU, UJUMP, RJUMP, CJUMP, SJUMP, FJUMP };
static struct {signed char regmap[HOST_REGS]; unsigned isconst;} regs[MAXBLOCK];
static int constmap[MAXBLOCK][HOST_REGS],bt[MAXBLOCK],itype[MAXBLOCK],slen;
static uint64_t unneeded_reg[MAXBLOCK],unneeded_reg_upper[MAXBLOCK];
''' + active + r'''
static void initialize(int reg, int hr, int size) {
 slen=size;
 for(int i=0;i<size;i++) {
  regs[i].regmap[hr]=(signed char)reg;
  regs[i].isconst=1u<<hr;
  constmap[i][hr]=1000+i;bt[i]=0;itype[i]=ALU;
  unneeded_reg[i]=unneeded_reg_upper[i]=UINT64_MAX;
 }
}
int main(int argc, char **argv) {
 (void)argv;
 if(argc>1) {
  // The actual caller admits upper-half constants when their low register is
  // not sign-extended. Stop before the end so the upper mask must be queried.
  initialize(65,0,2);regs[1].regmap[0]=66;int value=0;
  assert(get_final_value(0,0,&value)==0 && value==1000);
  return 0;
 }
 unsigned checked=0;
 for(int hr=0;hr<HOST_REGS;hr++) for(int reg=0;reg<128;reg++) {
  initialize(reg,hr,2);regs[1].regmap[hr]=(signed char)(reg^1);
  for(int needed=0;needed<2;needed++) {
   uint64_t mask=needed?0:UINT64_C(1)<<(reg&63);
   unneeded_reg[1]=unneeded_reg_upper[1]=mask;
   for(int branch=ALU;branch<=FJUMP;branch++) {
    itype[0]=branch;int value=-1;
    int expected=needed || (branch>=UJUMP && branch<=SJUMP);
    assert(get_final_value(hr,0,&value)==expected && value==1000);checked++;
   }
  }
  initialize(reg,hr,MAXBLOCK);
  int value=-1;
  assert(get_final_value(hr,0,&value)==1 && value==1000+MAXBLOCK-1);checked++;
  for(int boundary=1;boundary<8;boundary++) for(int kind=0;kind<3;kind++) {
   initialize(reg,hr,16);
   if(kind==0) bt[boundary]=1;
   if(kind==1) regs[boundary].isconst=0;
   if(kind==2) regs[boundary].regmap[hr]=(signed char)(reg^1);
   value=-1;
   assert(get_final_value(hr,0,&value)==0 && value==1000+boundary-1);checked++;
  }
 }
 printf("Actual constant reader preserves values/scan/branch rules across %u cases; no timing claim\n",checked);
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / ('constant-original.c' if before else 'constant.c')
    path.write_text(fixture)
    for suffix, flags in [('normal', []), ('trap', ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error'])]:
        if before and suffix == 'normal':
            continue
        exe = out / (path.stem + '-' + suffix)
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *flags,
                        str(path), '-o', str(exe)], check=True, timeout=25)
        result = subprocess.run([str(exe), 'upper-defect'] if before else [str(exe)], timeout=30)
        if before:
            assert result.returncode == -signal.SIGILL, 'original must trap the invalid upper-mask shift'
            print('Actual original upper-mask shift traps under UBSan; no native execution claim')
        else:
            result.check_returncode()


if __name__ == '__main__':
    main()
