#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare the actual culling helper with the retained native lookup contract."""
from pathlib import Path
import resource
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[2]
RELATIVE = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'
SOURCE = ROOT / '.local/source/mupen64plus-next'


def substitutions():
    """The two declared changes inside the compiler; preserve the loop's final hr."""
    branch = '''          for(hr=0;hr<HOST_REGS;hr++)
          {
            if(regmap_pre[i+2][hr]>=0&&get_reg(regs[i+2].regmap_entry,regmap_pre[i+2][hr])<0) nr&=~(1LL<<hr);
            //if((regmap_entry[i+2][hr])>=0) if(!((nr>>hr)&1)) DebugMessage(M64MSG_VERBOSE, "%x-bogus(%d=%d)",start+i*4,hr,regmap_entry[i+2][hr]);
          }
'''
    branch_new = '''          nr&=~ntx_missing_pre_regs(regmap_pre[i+2],regs[i+2].regmap_entry);
          hr=HOST_REGS;
'''
    following = '''      if(i<slen-1) {
        for(hr=0;hr<HOST_REGS;hr++) {
          if(regmap_pre[i+1][hr]>=0&&get_reg(regs[i+1].regmap_entry,regmap_pre[i+1][hr])<0) nr&=~(1LL<<hr);
'''
    following_new = '''      if(i<slen-1) {
        nr&=~ntx_missing_pre_regs(regmap_pre[i+1],regs[i+1].regmap_entry);
        for(hr=0;hr<HOST_REGS;hr++) {
'''
    return [(branch, branch_new), (following, following_new)]


def declared_body(original):
    for before, after in substitutions():
        assert original.count(before) == 1
        original = original.replace(before, after)
    return original


def function(source, signature):
    start = source.index(signature)
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    active = (SOURCE / RELATIVE).read_text()
    with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + RELATIVE))
        original = archive.extractfile(member).read().decode()
    signature = 'static signed char get_reg(signed char regmap[],int r)'
    getter = function(active, signature)
    assert getter == function(original, signature)
    helper = function(active, 'static uint64_t ntx_missing_pre_regs(')
    header = (SOURCE / 'mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.h').read_text()
    host = next(line for line in header.splitlines() if line.startswith('#define HOST_REGS '))
    exclude = next(line for line in header.splitlines() if line.startswith('#define EXCLUDE_REG '))
    fixture = '''
#include <assert.h>
#include <stdint.h>
#include <string.h>
''' + host + '\n#ifndef EXCLUDE_REG\n' + exclude + '\n#endif\n' + getter + '\n' + helper + r'''
static uint32_t random_state=0x835c29d1u;
static uint32_t random32(void) {
 random_state^=random_state<<13;random_state^=random_state>>17;random_state^=random_state<<5;
 return random_state;
}
static void check(signed char pre[],signed char entry[]) {
 signed char saved_pre[HOST_REGS],saved_entry[HOST_REGS];
 memcpy(saved_pre,pre,sizeof(saved_pre));memcpy(saved_entry,entry,sizeof(saved_entry));
 uint64_t seed=((uint64_t)random32()<<32)|random32(),expected=seed,actual=seed;
 int hr;
 for(hr=0;hr<HOST_REGS;hr++)
  if(pre[hr]>=0 && get_reg(entry,pre[hr])<0) expected&=~(UINT64_C(1)<<hr);
 int previous_hr=hr;
 actual&=~ntx_missing_pre_regs(pre,entry);hr=HOST_REGS;
 assert(actual==expected && hr==previous_hr);
 assert(!memcmp(pre,saved_pre,sizeof(saved_pre)) && !memcmp(entry,saved_entry,sizeof(saved_entry)));
}
int main(void) {
 signed char pre[HOST_REGS],entry[HOST_REGS];
 for(int code=-128;code<=127;code++) {
  for(int hr=0;hr<HOST_REGS;hr++) {pre[hr]=(signed char)code;entry[hr]=-1;}
  check(pre,entry);
  for(int slot=0;slot<HOST_REGS;slot++) {
   entry[slot]=(signed char)code;check(pre,entry);entry[slot]=-1;
  }
  memset(entry,code,sizeof(entry));check(pre,entry);check(entry,entry);
 }
 for(unsigned test=0;test<25000;test++) {
  for(int hr=0;hr<HOST_REGS;hr++) {
   pre[hr]=(signed char)((int)(random32()&255u)-128);
   entry[hr]=(signed char)((int)(random32()&255u)-128);
  }
  check(pre,entry);check(pre,pre);
 }
 return 0;
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / 'register-membership.c'
    path.write_text(fixture)
    for selection in [[], ['-DEXCLUDE_REG=18']]:
        for sanitizers in [[], ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error']]:
            executable = out / 'register-membership'
            subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *selection,
                            *sanitizers, str(path), '-o', str(executable)], check=True, timeout=20)
            subprocess.run([str(executable)], check=True, timeout=20)
    print('Actual culling membership matches retained lookup: all signed-byte codes/slots, exclusions, duplicates, changing maps and read-only aliasing; normal/UBSan trap')


if __name__ == '__main__':
    main()
