#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check the actual debug bridge without executing an unsafe format string."""
from pathlib import Path
import resource
import subprocess
import tarfile
from check_register_membership import function

ROOT = Path(__file__).resolve().parents[2]
REL = 'libretro/libretro.c'
SIGNATURE = 'static void n64DebugCallback('


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    source = (ROOT / '.local/source/mupen64plus-next' / REL).read_text()
    with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + REL))
        original = archive.extractfile(member).read().decode()
    old = function(original, SIGNATURE)
    actual = function(source, SIGNATURE)
    assert actual == old.replace('log_cb(RETRO_LOG_INFO, buffer);', 'log_cb(RETRO_LOG_INFO, "%s", buffer);')
    fixture = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#define CORE_NAME "mupen64plus"
#define RETRO_LOG_INFO 1
static bool literal;
static unsigned calls;
static char observed[1024];
static void sink(int level,const char *format,...) {
 assert(level==RETRO_LOG_INFO);calls++;
 literal=!strcmp(format,"%s");
 // Deliberately refuse the old bridge before interpreting any unsafe payload.
 if(!literal)return;
 va_list args;va_start(args,format);
 vsnprintf(observed,sizeof(observed),format,args);va_end(args);
}
static void (*log_cb)(int,const char*,...)=sink;
''' + old.replace('n64DebugCallback(', 'originalDebugCallback(', 1) + '\n' + actual + r'''
int main(void) {
 void (*volatile bridge)(void*,int,const char*)=n64DebugCallback;
 const char *payloads[]={"","ordinary text","progress100%: %s %n %p %%","%%n"};
 for(unsigned i=0;i<sizeof(payloads)/sizeof(payloads[0]);i++) {
  char expected[1024];snprintf(expected,sizeof(expected),CORE_NAME ": %s\n",payloads[i]);
  calls=0;literal=false;bridge(NULL,0,payloads[i]);
  assert(calls==1&&literal&&!strcmp(observed,expected));
 }
 calls=0;literal=true;originalDebugCallback(NULL,0,"progress100%: %s %n %p %%");
 assert(calls==1&&!literal);
 char long_message[4096],expected[1024];memset(long_message,'%',sizeof(long_message));long_message[4095]=0;
 memset(expected,'%',1023);memcpy(expected,CORE_NAME ": ",sizeof(CORE_NAME ": ")-1);expected[1023]=0;
 calls=0;bridge(NULL,0,long_message);
 assert(calls==1&&literal&&strlen(observed)==1023&&!strcmp(observed,expected));
 log_cb=NULL;calls=0;bridge(NULL,0,"%s %n");assert(calls==0);
 return 0;
}
'''
    out = ROOT / 'build/ntx-mupen-boundary'
    out.mkdir(parents=True, exist_ok=True)
    path = out / 'log-payload.c'
    path.write_text(fixture)
    for suffix, flags in [('normal', []), ('trap', ['-fsanitize=undefined', '-fsanitize-undefined-trap-on-error'])]:
        exe = out / ('log-payload-' + suffix)
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', '-Wno-unused-parameter', *flags, str(path), '-o', str(exe)], check=True, timeout=20)
        subprocess.run([str(exe)], check=True, timeout=10)
    print('Actual debug bridge preserves literal percent directives, bounded truncation and absent sink; original rejected before unsafe formatting, normal/UBSan trap')


if __name__ == '__main__':
    main()
