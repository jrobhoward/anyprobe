# Draft: SystemTap misplaces SDT probes in lld-linked executables

Not yet filed. Target: <https://sourceware.org/bugzilla/> (product
`systemtap`, component `runtime` or `translator`).

Before filing, run the C reproducer below under `stap` on both binaries and
replace the "C reproducer result" placeholder with the output. The Rust
results are from real runs; the C result has not been run yet.

---

## Summary

On an executable linked by lld, `process(...).mark(...)` probes are
registered at the wrong file offset and the build-id check reads the wrong
address. lld places the executable segment at a file offset that differs from
its virtual address (here by 0x1000); GNU ld keeps them equal. SystemTap
appears to treat the probe's virtual address as the inode offset. The same
binaries work under bpftrace and perf.

## Environment

- SystemTap 5.6, built from the release tarball
- Ubuntu 24.04, kernel 6.8.0-138-generic, x86_64
- lld 22.1.2 (as shipped with Rust 1.95 as `rust-lld`, the default linker for
  `x86_64-unknown-linux-gnu`)

## Layout

C program using `<sys/sdt.h>`, built with gcc:

```
$ readelf -lW repro-bfd | grep 'R E'      # GNU ld
  LOAD  0x001000 0x0000000000001000 0x0000000000001000 0x000189 0x000189 R E 0x1000
$ readelf -lW repro-lld | grep 'R E'      # lld
  LOAD  0x000690 0x0000000000001690 0x0000000000001690 0x000190 0x000190 R E 0x1000
```

The SDT note in `repro-lld` records the probe at virtual address 0x1790,
which is file offset 0x790.

## Reproducer

```c
// repro.c
#include <stdio.h>
#include <unistd.h>
#include <sys/sdt.h>

int main(void) {
    for (long i = 0;; i++) {
        DTRACE_PROBE1(repro, tick, i);
        usleep(20000);
    }
}
```

```
gcc -O2 repro.c -o repro-bfd
gcc -O2 -fuse-ld=lld repro.c -o repro-lld

./repro-lld & pid=$!
sudo stap -x $pid -e 'probe process(@1).mark("tick") { printf("%d\n", $arg1); exit() }' \
  "$PWD/repro-lld"
kill $pid
```

Expected: one line with the loop counter, as with `repro-bfd`.

C reproducer result: _(to be filled in)_

## Observed (Rust executable, same cause)

An executable with SDT notes and semaphores, linked by rust-lld. Text segment
at file offset 0xdeb0, virtual address 0xeeb0. Probe sites at virtual
addresses 0xf04c, 0xf05e, 0x11240 and 0x11250 (file offsets 0x1000 lower).

```
WARNING: probe process(".../anyprobe-spike").statement(0xf04c) at inode-offset 59542105:00000000212471ed registration error [man warning::pass5] (rc -524)
WARNING: probe process(".../anyprobe-spike").statement(0x11240) at inode-offset 59542105:000000001c97a4e8 registration error [man warning::pass5] (rc -524)
WARNING: Build-id mismatch [man warning::buildid]: ".../anyprobe-spike" pid 73784 address 0x645fc430632c, expected 684327cf08d484c8a3514d7e234f7ec04503ec41 actual 00000000936300000000000040b6040000000000
WARNING: task_finder mmap inode-uprobes callback for task 73784 failed: 1
```

`-524` is `ENOTSUPP` from uprobes: the offset given does not start a
supported instruction. The two other sites register at the wrong place, and
no probe fires. The `.note.gnu.build-id` section is at virtual address and
file offset 0x31c (descriptor at 0x32c), in the first segment, where the two
agree. The address read is still wrong, so the base used for the build-id
check seems to come from the executable mapping as well.

Semaphores (in `.probes`) are raised and cleared correctly.

## Workarounds that confirm the cause

- Linking the same program with `-Wl,-z,separate-loadable-segments` (lld
  pads the file so every segment's offset equals its address): all probes
  register, fire, and read their arguments, in both a release and a fat-LTO
  build.
- bpftrace 0.20.2 and perf 6.8.12 attach to the unpadded binary. bpftrace
  converts addresses through the program headers. perf converts through the
  `.stapsdt.base` section's offset, which works once that section is in the
  executable segment.

## Suggested fix

Convert the note's (prelink-adjusted) virtual address to a file offset
through the program header of the `PT_LOAD` segment that contains it
(`addr - p_vaddr + p_offset`), rather than assuming offset equals address;
compute the load base the same way for the build-id check.
