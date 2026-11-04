#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compile the actual pinned state entrypoints and ABI against bounded stubs.

This verifies callback/results/buffer admission and the completed-job yield;
it does not claim console execution, a GPU, or successful game-state restore.
"""
from pathlib import Path
import subprocess
import sys
import tarfile
import hashlib
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / '.local/source/mupen64plus-next'
OUT = ROOT / 'build/ntx-mupen-boundary'
OUT.mkdir(parents=True, exist_ok=True)


def function(text, signature):
    start = text.index(signature)
    brace = text.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (text[end] == '{') - (text[end] == '}')
        end += 1
    return text[start:end]


libretro = (SOURCE / 'libretro/libretro.c').read_text()
state = (SOURCE / 'mupen64plus-core/src/main/savestates.c').read_text()
parts = [function(libretro, 'size_t retro_serialize_size ('),
         function(libretro, 'bool retro_serialize('),
         function(libretro, 'bool retro_unserialize('),
         function(libretro, 'void retro_deinit(void)'),
         function(libretro, 'RETRO_API bool retro_ntx_shutdown_complete(void)')]
for operation in ['load', 'save']:
    body = function(state, f'int savestates_{operation}(void)')
    tail = body[body.rindex('    savestates_clear_job();'):]
    parts.append(f'int checked_{operation}_boundary(void){{int ret=1;\n' + tail)

fixture = r'''
#include <assert.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "libretro.h"
_Static_assert(sizeof(struct retro_hw_render_callback)==64,"hardware ABI");
_Static_assert(offsetof(struct retro_hw_render_callback,version_major)==36,"version ABI");
_Static_assert(offsetof(struct retro_hw_render_callback,context_destroy)==48,"destroy ABI");
_Static_assert(sizeof(struct retro_core_option_v2_definition)==2104,"option ABI");
_Static_assert(sizeof(struct retro_game_info)==32,"content ABI");
_Static_assert(sizeof(struct retro_system_av_info)==40,"AV ABI");
enum {RDP_PLUGIN_GLIDEN64=1,GLSM_CTL_STATE_BIND=1,GLSM_CTL_STATE_UNBIND=2,
      savestates_job_save=1,savestates_job_load=2,savestates_type_m64p=1};
static bool initializing,retro_savestate_complete;
static int retro_savestate_result,current_rdp_type=1,EnableThreadedRenderer;
static int desired_result,submitted,switched,job,yielded,bound;
static void *game_thread;
static void savestates_set_job(int j,int t,const void *p){assert(t==1&&p);job=j;submitted++;}
static bool deinit_mode,acknowledge_stop;
static int execution_state,shutdowns,deletes,stops,closes;
static void co_switch(void *thread){(void)thread;switched++;retro_savestate_complete=true;retro_savestate_result=desired_result;if(deinit_mode&&acknowledge_stop)execution_state=0;}
static void glsm_ctl(int operation,void *p){(void)p;bound+=(operation==1?1:-1);assert(bound>=0);}
static void environ_clear_thread_waits_cb(int on,void *p){(void)on;(void)p;}
static void savestates_clear_job(void){job=0;}
static void retro_return(void){assert(job==0);yielded++;}
enum {M64CMD_STOP=10,M64CMD_CORE_STATE_QUERY=11,M64CMD_ROM_CLOSE=12,M64CORE_EMU_STATE=1,M64ERR_SUCCESS=0,M64EMU_RUNNING=1,M64EMU_STOPPED=0};
static bool ntx_shutdown_complete,load_game_successful;
static char rdp_plugin_last[2];
static int CoreOptionCategoriesSupported,CoreOptionUpdateDisplayCbSupported;
static struct {void (*perf_log)(void);} perf_cb;
static int CoreDoCommand(int command,int argument,void *data){(void)argument;switch(command){case 10:stops++;break;case 11:*(int*)data=execution_state;break;case 12:closes++;break;}return 0;}
static void CoreShutdown(void){shutdowns++;}
static void deinit_audio_libretro(void){}
static void co_delete(void *thread){assert(thread);deletes++;}
''' + '\n'.join(parts) + r'''
int main(void){
 char bytes[1];size_t n=retro_serialize_size();
 initializing=true;assert(!retro_serialize(bytes,n));assert(!retro_unserialize(bytes,n));
 initializing=false;assert(!retro_serialize(NULL,n));assert(!retro_unserialize(NULL,n));
 assert(!retro_serialize(bytes,n-1));assert(!retro_unserialize(bytes,n-1));assert(submitted==0&&switched==0);
 desired_result=0;assert(!retro_serialize(bytes,n));assert(!retro_unserialize(bytes,n));
 desired_result=1;assert(retro_serialize(bytes,n));assert(retro_unserialize(bytes,n));
 assert(submitted==4&&switched==4&&bound==0);
 job=1;assert(checked_save_boundary()==1);job=2;assert(checked_load_boundary()==1);assert(yielded==2);
 deinit_mode=true;load_game_successful=true;initializing=true;game_thread=(void*)1;
 int before=switched;retro_deinit();assert(switched==before&&shutdowns==1&&deletes==1&&stops==0&&closes==1&&retro_ntx_shutdown_complete());
 initializing=false;game_thread=(void*)1;ntx_shutdown_complete=false;execution_state=1;acknowledge_stop=false;
 retro_deinit();assert(shutdowns==1&&deletes==1&&closes==1&&!retro_ntx_shutdown_complete()&&game_thread);
 acknowledge_stop=true;retro_deinit();assert(shutdowns==2&&deletes==2&&closes==2&&retro_ntx_shutdown_complete()&&!game_thread);
 return 0;
}
'''
path = OUT / 'boundary.c'
path.write_text(fixture)
executable = OUT / 'boundary'
subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-D__LIBRETRO__',
                '-I', str(SOURCE / 'libretro-common/include'), str(path), '-o', str(executable)], check=True)
subprocess.run([str(executable)], check=True)
print('Actual libretro ABI, checked state admission/results and completed-job yields passed; no emulation claim')

# Reproduce the maintained borrowed-file changes from the exact retained archive.
patch_temporary = tempfile.TemporaryDirectory(prefix='ntx-mupen-patch-')
patch_root = Path(patch_temporary.name)
with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
    for name in ['libretro/libretro.c', 'mupen64plus-core/src/main/savestates.c', 'GLideN64/src/Graphics/OpenGLContext/GLFunctions.cpp', 'custom/GLideN64/mupenplus/CommonAPIImpl_mupenplus.cpp', 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c', 'mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.c']:
        member = next(m for m in archive.getmembers() if m.name.endswith('/' + name))
        destination = patch_root / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(archive.extractfile(member).read())
for name in ['0001-checked-state-result.patch', '0002-coherent-state-boundary.patch', '0003-host-shader-dispatch.patch', '0004-profile-shader-cache.patch', '0005-compile-observer.patch', '0006-compiler-phase-observer.patch', '0007-complete-cache-batch.patch', '0008-cull-register-membership.patch', '0009-cull-query-counts.patch', '0010-incoming-branch-index.patch', '0011-literal-log-payload.patch', '0012-assembly-child-scopes.patch', '0013-upper-constant-mask.patch']:
    subprocess.run(['git', 'apply', '--no-index', str(ROOT / 'native/mupen' / name)], cwd=patch_root, check=True)
for name in ['libretro/libretro.c', 'mupen64plus-core/src/main/savestates.c', 'GLideN64/src/Graphics/OpenGLContext/GLFunctions.cpp', 'custom/GLideN64/mupenplus/CommonAPIImpl_mupenplus.cpp', 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c', 'mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.c']:
    assert hashlib.sha256((patch_root / name).read_bytes()).digest() == hashlib.sha256((SOURCE / name).read_bytes()).digest()
print('All maintained patches reproduce the exact active source bytes')
patch_temporary.cleanup()

# Check exact wrapper/registrar and the declared culling substitutions.
compiler_path = 'mupen64plus-core/src/device/r4300/new_dynarec/new_dynarec.c'
compiler = (SOURCE / compiler_path).read_text()
with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
    member = next(m for m in archive.getmembers() if m.name.endswith('/' + compiler_path))
    original = archive.extractfile(member).read().decode()
# This pinned compiler is the final function; preprocessing alternatives contain
# unmatched braces before preprocessing, so compare its complete retained tail.
original_body = original[original.index('int new_recompile_block(int addr)\n{'):].split('{', 1)[1].strip()
active_body = compiler[compiler.index('static int ntx_recompile_block(int addr)\n{'):compiler.index('/* Mechanical boundary:')].split('{', 1)[1].strip()
from check_register_membership import declared_body as declared_membership
from check_culling_queries import declared_body as declared_queries
from check_incoming_branches import declared_body as declared_incoming
assert declared_incoming(declared_queries(declared_membership(original_body))) == ''.join(line for line in active_body.splitlines(True) if not line.strip().startswith(('NTX_PHASE_EVENT(', 'NTX_ASM_EVENT('))).strip()
bridge_start = libretro.index('typedef void (*ntx_compile_observer_t)')
bridge_end = libretro.index('static bool ntx_shutdown_complete', bridge_start)
compile_fixture = r"""
#include <assert.h>
#include <stddef.h>
#define RETRO_API
static int answer, seen_address, begins, ends;
static int ntx_recompile_block(int addr) { seen_address=addr; return answer; }
""" + libretro[bridge_start:bridge_end] + function(compiler, 'int new_recompile_block(int addr)\n{') + r"""
static void observe(unsigned entering) { if (entering) begins++; else ends++; }
int main(void) {
 retro_ntx_set_compile_observer(observe);
 answer=0; assert(new_recompile_block(123)==0 && seen_address==123);
 answer=1; assert(new_recompile_block(-71)==1 && seen_address==-71);
 assert(begins==2 && ends==2);
 retro_ntx_set_compile_observer(NULL);
 answer=-9; assert(new_recompile_block(56)==-9 && seen_address==56);
 assert(begins==2 && ends==2);
 return 0;
}
"""
path = OUT / 'compile-observer.c'
path.write_text(compile_fixture)
executable = OUT / 'compile-observer'
subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-D__LIBRETRO__', str(path), '-o', str(executable)], check=True)
subprocess.run([str(executable)], check=True)
print('Compiler remainder retained outside observations and declared analysis substitutions; exact observer ABI preserves arguments/results and nullable retirement')

# Observation statements are the only changes inside the cache primitive too.
cache_path = 'mupen64plus-core/src/device/r4300/new_dynarec/arm64/assem_arm64.c'
with tarfile.open(ROOT / '.local/source-archives/mupen64plus-next-15bada85.tar.gz') as archive:
    member = next(m for m in archive.getmembers() if m.name.endswith('/' + cache_path))
    original_cache = archive.extractfile(member).read().decode()
active_cache = (SOURCE / cache_path).read_text()
cache_original_body = function(original_cache, 'static void cache_flush(char* start, char* end)').split('{', 1)[1]
cache_active_body = function(active_cache, 'static void cache_flush(char* start, char* end)').split('{', 1)[1]
assert cache_original_body == ''.join(line for line in cache_active_body.splitlines(True) if not line.strip().startswith(('NTX_PHASE_EVENT(', 'NTX_CACHE_BEGIN(')))
print('Compiler remainder checked outside declared analysis substitutions; cache primitive operations/barriers retained outside notifications, batch scheduling checked separately')

subprocess.run(['python3', str(ROOT / 'native/mupen/check_cache_batch.py')], check=True)

subprocess.run(['python3', str(ROOT / 'native/mupen/check_register_membership.py')], check=True)

subprocess.run(['python3', str(ROOT / 'native/mupen/check_culling_queries.py')], check=True)

subprocess.run(['python3', str(ROOT / 'native/mupen/check_incoming_branches.py')], check=True)

subprocess.run(['python3', str(ROOT / 'native/mupen/check_log_payload.py')], check=True)

subprocess.run([sys.executable, str(ROOT / "native/mupen/check_assembly_scopes.py")], check=True)

subprocess.run([sys.executable, str(ROOT / "native/mupen/check_final_constant.py"), "--original"], check=True)
subprocess.run([sys.executable, str(ROOT / "native/mupen/check_final_constant.py")], check=True)
