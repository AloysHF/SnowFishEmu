"""Bisects the reference emulator's LZ state against my decoder's boundaries:
for each boundary L (input position), builds prefix[0..L] + terminator; if
the reference's decode state is in sync at L, its out_len must equal my
out(L). First mismatch localizes the semantic divergence."""
import struct
import sys
import pickle

sys.path.insert(0, r'D:\Code\Emulator\SnowFishEmu\tmp')
import build_oracle_app as B

out, bounds = pickle.load(open('hjkg_bounds.pkl', 'rb'))
src = open('SF_DUMP_BLOCK_0011B7FA_7311.bin', 'rb').read()
# out_at_ip: map ip -> my out length at that boundary
ip2out = dict((ip, o) for ip, o in bounds)


def prefix_app(ip):
    stream = src[:ip] + bytes([0x11, 0x00, 0x00])
    return B.build([stream])[0]


def run(app, name):
    open(f'ref-run/rom/bisect_{name}.app', 'wb').write(app)
    import subprocess
    r = subprocess.run(
        ['sfemu.exe', '--headless', '-v', f'rom/bisect_{name}.app'],
        capture_output=True, timeout=30, cwd='ref-run')
    txt = (r.stdout + r.stderr).decode('utf-8', 'replace')
    for line in txt.splitlines():
        if '-> ret' in line:
            ret = line.split('-> ret')[1].strip()
            return ret
    return '?'


def main():
    targets = [ip for ip, o in bounds if ip <= 620]
    results = []
    lo_ok = None
    for ip in targets[:40]:
        ret = run(prefix_app(ip), f'ip{ip}')
        mine = ip2out.get(ip, -1)
        # reference out_len extracted from ret line "out_len=N"
        import re
        m = re.search(r'out_len=(\d+)', ret)
        refout = int(m.group(1)) if m else -1
        status = 'SYNC' if refout == mine else 'DIFF'
        results.append((ip, mine, ret, status))
        print(f'ip={ip} mine_out={mine} ref={ret} {status}')
        if status == 'DIFF':
            break


if __name__ == '__main__':
    main()
