#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise the actual selected native dirty-page batcher, without ARM execution."""
from pathlib import Path
import subprocess
import resource
resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
ROOT=Path(__file__).resolve().parents[2]
source=(ROOT/'.local/source/mupen64plus-next/mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.c').read_text()
start=source.index('static void do_clear_cache(void)');brace=source.index('{',start);end=brace+1;depth=1
while depth:
 depth+=(source[end]=='{')-(source[end]=='}');end+=1
body=source[start:end]
fixture=r'''
#include <assert.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
typedef unsigned int u_int;
#define TARGET_SIZE_2 18
static _Alignas(4096) unsigned char arena[1u<<TARGET_SIZE_2];
static unsigned char *base_addr_rx=arena;
static unsigned int needs_clear_cache[2];
static uint64_t seen;
static unsigned int calls, tail_calls;
#define NTX_CACHE_TAIL() (tail_calls++)
static void cache_flush(char *start, char *end) {
 uintptr_t lo=(uintptr_t)start-(uintptr_t)arena;
 uintptr_t hi=(uintptr_t)end-(uintptr_t)arena;
 assert(lo<(1u<<TARGET_SIZE_2) && hi<(1u<<TARGET_SIZE_2) && lo<=hi);
 assert(lo%4096==0 && hi%4096==4095);
 for(unsigned page=(unsigned)(lo/4096);page<=hi/4096;page++) {
  assert(needs_clear_cache[page/32] & (1u<<(page%32)));
  seen|=UINT64_C(1)<<page;
 }
 calls++;
}
''' + body + r'''
static void check(unsigned int low,unsigned int high) {
 needs_clear_cache[0]=low;needs_clear_cache[1]=high;seen=0;calls=0;tail_calls=0;
 uint64_t expected=((uint64_t)high<<32)|low;
 do_clear_cache();
 assert(seen==expected);
 assert(tail_calls==(low>>31)+(high>>31));
 assert(needs_clear_cache[0]==0 && needs_clear_cache[1]==0);
 assert((expected==0)==(calls==0));
}
int main(void) {
 for(unsigned bit=0;bit<64;bit++) check(bit<32?1u<<bit:0,bit>=32?1u<<(bit-32):0);
 check(0xffffffffu,0xffffffffu);
 check(0xffff0000u,0x80000000u);
 check(0x80000001u,0xaaaaaaaaU);
 check(0x55555555u,0x7fffffffu);
 check(0,0);
 return 0;
}
'''
out=ROOT/'build/ntx-mupen-boundary';out.mkdir(parents=True,exist_ok=True)
p=out/'cache-batch.c';p.write_text(fixture);exe=out/'cache-batch'
for flags in [[], ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error']]:
 subprocess.run(['cc','-std=c11','-Wall','-Wextra','-Werror',*flags,str(p),'-o',str(exe)],check=True)
 subprocess.run([str(exe)],check=True)
print('Actual native batcher covers every dirty page, including bit31/full tails, before clearing its mask')
