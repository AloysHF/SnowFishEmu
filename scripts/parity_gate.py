"""Parity gate: runs every corpus game on BOTH the reference emulator and
SnowFishEmu headlessly, and reports side-by-side status.

A game is at parity when both implementations show the same failure mode
or the same progress signature (tick completions, service-call volume)."""
import re
import subprocess
import sys
from pathlib import Path

CORPUS = Path(r'D:\Code\Emulator\SnowFishEmu\tmp\snowfish_game')
MINE = Path(r'D:\Code\Emulator\SnowFishEmu\target\debug\sfheadless.exe')
REF = Path(r'D:\Code\Emulator\SnowFishEmu\tmp\ref-run\sfemu.exe')
OUT = Path(r'D:\Code\Emulator\SnowFishEmu\tmp\parity')
FRAMES = 30


def run_mine(main, res_arg, log):
    with open(log, 'wb') as lf:
        try:
            subprocess.run([str(MINE), str(main), *res_arg,
                            '--frames', str(FRAMES), '--budget', '100000000'],
                           timeout=110, stdout=lf, stderr=lf)
            status = 'ran'
        except subprocess.TimeoutExpired:
            status = 'timeout'
    txt = log.read_text(encoding='utf-8', errors='replace')
    counters = re.search(r'gdi counters \[([^\]]*)\]', txt)
    counters = counters.group(1) if counters else '-'
    lz_ok = len(re.findall(r'lz -> ret=0', txt))
    lz_fail = len(re.findall(r'lz failed', txt))
    faults = len(re.findall(r'guest call faulted|guest executing at 0x', txt))
    return (status, lz_ok, lz_fail, faults, counters)


def run_ref(main, log):
    exe_dir = REF.parent
    rom = exe_dir / 'rom' / main.name
    rom.write_bytes(main.read_bytes())
    with open(log, 'wb') as lf:
        try:
            subprocess.run([str(REF), '--headless', '-v', f'rom/{main.name}'],
                           timeout=90, stdout=lf, stderr=lf, cwd=exe_dir)
            status = 'ran'
        except subprocess.TimeoutExpired:
            status = 'timeout'
    txt = log.read_text(encoding='utf-8', errors='replace')
    m = re.search(r'\[headless\] INIT OK.*?fopen=(\d+) fread=(\d+) lz=(\d+) '
                  r'log=(\d+) fseek=(\d+) fclose=(\d+)', txt)
    if m:
        return ('INIT-OK', f'fopen={m.group(1)} lz={m.group(3)} log={m.group(4)}')
    fails = re.findall(r'msg=\S+ emu failed', txt)
    ticks = re.findall(r'\[headless\] TICK \d+ FAILED', txt)
    return ('FAILED', f'emufails={len(fails)} tickfails={len(ticks)}')


def main():
    OUT.mkdir(exist_ok=True)
    print(f"{'game':<28} | {'mine':<38} | reference")
    print('-' * 110)
    for game_dir in sorted(CORPUS.iterdir()):
        if not game_dir.is_dir():
            continue
        apps = sorted(game_dir.glob('*.app'))
        mains = [a for a in apps if 'res' not in a.stem.lower()]
        if not mains:
            continue
        main = mains[0]
        res = [a for a in apps if 'res' in a.stem.lower()]
        res_arg = []
        if res:
            combined = OUT / (main.stem + '_res.bin')
            combined.write_bytes(b''.join(r.read_bytes() for r in sorted(res)))
            res_arg = ['--res', str(combined)]
        mine = run_mine(main, res_arg, OUT / (game_dir.name + '.mine.log'))
        ref = run_ref(main, OUT / (game_dir.name + '.ref.log'))
        m = (f'{mine[0]} lz_ok={mine[1]} lz_fail={mine[2]} faults={mine[3]} '
             f'[{mine[4]}]')
        print(f'{game_dir.name:<28} | {m:<38} | {ref[0]} {ref[1]}')


if __name__ == '__main__':
    main()
