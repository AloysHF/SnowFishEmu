"""Batch gate: runs every game in the corpus headlessly and reports status.

Success criteria per game: boots, no lz/emu failures in the window, and a
framebuffer that is not empty. Writes a summary table.
"""
import re
import subprocess
import sys
from pathlib import Path

CORPUS = Path(r'D:\Code\Emulator\SnowFishEmu\tmp\snowfish_game')
EMU = Path(r'D:\Code\Emulator\SnowFishEmu\target\debug\sfheadless.exe')
OUT = Path(r'D:\Code\Emulator\SnowFishEmu\tmp\gate')
FRAMES = 30


def main():
    OUT.mkdir(exist_ok=True)
    rows = []
    for game_dir in sorted(CORPUS.iterdir()):
        if not game_dir.is_dir():
            continue
        apps = sorted(game_dir.glob('*.app'))
        # main executable = the one NOT containing "Res"
        mains = [a for a in apps if 'Res' not in a.stem and 'res' not in a.stem]
        res = [a for a in apps if 'Res' in a.stem or 'res' in a.stem]
        if not mains:
            rows.append((game_dir.name, 'NO-MAIN', '', ''))
            continue
        main = mains[0]
        res_arg = []
        if res:
            # concatenate companion resource files if several
            combined = OUT / (main.stem + '_res.bin')
            combined.write_bytes(b''.join(r.read_bytes() for r in sorted(res)))
            res_arg = ['--res', str(combined)]
        log = OUT / (game_dir.name + '.log')
        with open(log, 'wb') as lf:
            try:
                subprocess.run(
                    [str(EMU), str(main), *res_arg,
                     '--frames', str(FRAMES), '--budget', '100000000'],
                    timeout=110, stdout=lf, stderr=lf)
                status = 'ran'
            except subprocess.TimeoutExpired:
                status = 'timeout'
        txt = log.read_text(encoding='utf-8', errors='replace')
        counters = re.search(r'gdi counters \[([^\]]*)\]', txt)
        counters = counters.group(1) if counters else '?'
        lz_fail = len(re.findall(r'lz failed', txt))
        emu_fail = len(re.findall(r'emu failed|GuestFault|guest fault', txt))
        guest_fb = re.search(r'guest fb nonzero: (\d+) / (\d+)', txt)
        nonzero = guest_fb.group(1) if guest_fb else '?'
        rows.append((game_dir.name, status,
                     f'lzfail={lz_fail} emufail={emu_fail}',
                     f'gdfb_nz={nonzero} [{counters}]'))
        print(f'{game_dir.name}: {status} lzfail={lz_fail} emufail={emu_fail} '
              f'guestfb_nz={nonzero} [{counters}]')
    print('\n=== summary ===')
    for r in rows:
        print(' | '.join(str(x) for x in r))


if __name__ == '__main__':
    main()
