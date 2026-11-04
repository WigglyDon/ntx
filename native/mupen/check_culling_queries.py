#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Actual culling query counts vs the retained getter across live map erases."""
from pathlib import Path
import re
import resource
import subprocess
from check_register_membership import function

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / '.local/source/mupen64plus-next'


def declared_body(original):
    start = original.index('  /* Pass 4 - Cull unused host registers */')
    end = original.index('  /* Pass 5 - Pre-allocate registers */', start)
    body = original[start:end]
    anchor = '    // Deallocate unneeded registers\n'
    assert body.count(anchor) == 1
    body = body.replace(anchor, '''    // Only count a query when first needed; subsequent erases keep it exact.
    int ntx_query=((itype[i]==RJUMP||itype[i]==UJUMP||itype[i]==CJUMP||itype[i]==SJUMP||itype[i]==FJUMP)?rt1[i+1]:rt1[i])|64;
    unsigned ntx_current_matches=~0u,ntx_branch_matches=~0u;
''' + anchor)
    before = 'if(get_reg(regs[i].regmap,rt1[i+1]|64)>=0||get_reg(branch_regs[i].regmap,rt1[i+1]|64)>=0)'
    assert body.count(before) == 1
    body = body.replace(before, 'if(ntx_query_present(regs[i].regmap,ntx_query,&ntx_current_matches)||ntx_query_present(branch_regs[i].regmap,ntx_query,&ntx_branch_matches))')
    before = 'if(get_reg(regs[i].regmap,rt1[i]|64)>=0)'
    assert body.count(before) == 1
    body = body.replace(before, 'if(ntx_query_present(regs[i].regmap,ntx_query,&ntx_current_matches))')
    body, count = re.subn(r'(?<![A-Za-z_])regs\[i\]\.regmap\[hr\]=-1;',
                         r'ntx_erase_queried_reg(regs[i].regmap,hr,ntx_query,&ntx_current_matches);', body)
    assert count == 3
    body, count = re.subn(r'(?<![A-Za-z_])branch_regs\[i\]\.regmap\[hr\]=-1;',
                         r'ntx_erase_queried_reg(branch_regs[i].regmap,hr,ntx_query,&ntx_branch_matches);', body)
    assert count == 1
    return original[:start] + body + original[end:]


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    source = (SOURCE / 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c').read_text()
    header = (SOURCE / 'mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.h').read_text()
    host = next(s for s in header.splitlines() if s.startswith('#define HOST_REGS '))
    exclude = next(s for s in header.splitlines() if s.startswith('#define EXCLUDE_REG '))
    fixture = '#include <assert.h>\n#include <stdint.h>\n#include <string.h>\n' + host + '\n#ifndef EXCLUDE_REG\n' + exclude + '\n#endif\n'
    for signature in ('static signed char get_reg(signed char regmap[],int r)',
                      'static unsigned ntx_reg_matches(', 'static int ntx_query_present(', 'static void ntx_erase_queried_reg('):
        fixture += function(source, signature) + '\n'
    fixture += r'''
static uint32_t state=0xe738ab51u;
static uint32_t random32(void) {state^=state<<13;state^=state>>17;state^=state<<5;return state;}
static void check(signed char a[],signed char b[],int key) {
 signed char old_a[HOST_REGS],old_b[HOST_REGS];
 memcpy(old_a,a,sizeof(old_a));memcpy(old_b,b,sizeof(old_b));
 unsigned ca=~0u,cb=~0u;
 for(unsigned step=0;step<100;step++) {
  int hr=(int)(random32()%HOST_REGS);
  if(random32()&1) {ntx_erase_queried_reg(a,hr,key,&ca);old_a[hr]=-1;}
  else {ntx_erase_queried_reg(b,hr,key,&cb);old_b[hr]=-1;}
  // Erases can precede either map's first query. Keep short-circuit queries too.
  if(step>10 && (random32()&1)) assert(ntx_query_present(a,key,&ca)==(get_reg(old_a,key)>=0));
  if(step>20 && (random32()&1)) assert(ntx_query_present(b,key,&cb)==(get_reg(old_b,key)>=0));
  assert(ca==~0u || ca==ntx_reg_matches(a,key));
  assert(cb==~0u || cb==ntx_reg_matches(b,key));
  if(step>30) assert((ntx_query_present(a,key,&ca)||ntx_query_present(b,key,&cb))==((get_reg(old_a,key)>=0)||(get_reg(old_b,key)>=0)));
  assert(!memcmp(a,old_a,sizeof(old_a)) && !memcmp(b,old_b,sizeof(old_b)));
 }
}
int main(void) {
 signed char a[HOST_REGS],b[HOST_REGS];
 for(int key=-129;key<=128;key++) for(unsigned pattern=0;pattern<4;pattern++) {
  for(int hr=0;hr<HOST_REGS;hr++) {a[hr]=pattern==0?-1:pattern==1?(signed char)key:(signed char)hr;b[hr]=pattern==2?-1:(signed char)key;}
  check(a,b,key);
 }
 for(unsigned test=0;test<10000;test++) {
  for(int hr=0;hr<HOST_REGS;hr++) {a[hr]=(signed char)((int)(random32()&255)-128);b[hr]=(signed char)((int)(random32()&255)-128);}
  check(a,b,(int)(random32()%258)-129);
 }
 return 0;
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / 'culling-queries.c'
    path.write_text(fixture)
    for selection in [[], ['-DEXCLUDE_REG=18']]:
        for flags in [[], ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error']]:
            executable = out / 'culling-queries'
            subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', *selection,
                            *flags, str(path), '-o', str(executable)], check=True, timeout=20)
            subprocess.run([str(executable)], check=True, timeout=20)
    print('Actual culling query counts preserve retained getter through >1million live-map steps per mode, lazy/short-circuit queries, negative/free/duplicate/excluded values; normal/UBSan trap')


if __name__ == '__main__':
    main()
