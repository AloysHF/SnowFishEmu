import itertools

class LZErr(Exception):
    pass


def lz(src, src_len, window_prev=b'', P=None):
    P = P or {}
    w_cnt = P.get('w_cnt', 'b0')      # 'b0' | 'tok' | 'zero'
    a2_cnt = P.get('a2_cnt', 'b0')    # 'b0' | 'tok' | 'zero'
    w_adj = P.get('w_adj', 0x4000)    # W offset subtraction
    w_b3 = P.get('w_b3', 0x4000)      # (F&8) multiplier
    mid = P.get('mid', 0x801)
    x_cnt = P.get('x_cnt', 'tok')     # 'tok' | 'b0' | 'zero'
    x_nib = P.get('x_nib', 'r2m7')    # 'r2m7' ((F>>2)&7) | 'm7' (F&7) | 'r3m7' ((F>>3)&7) | 'm15' (F&15)
    x_sh = P.get('x_sh', 3)           # ob shift
    out = bytearray()
    prev = bytes(window_prev)
    prev_len = len(prev)

    def rd(i):
        if i < 0:
            j = prev_len + i
            return prev[j] if 0 <= j < prev_len else 0
        if i >= len(out):
            raise LZErr('fwd')
        return out[i]

    def rs(i):
        if i >= src_len:
            raise LZErr('over')
        return src[i]

    ip = 0
    F = src[0]
    ip = 1
    if F > 0x11:
        for _ in range(F - 0x11):
            out.append(rs(ip)); ip += 1
        F = rs(ip); ip += 1
    while True:
        if F <= 0x11 and F <= 0xF:
            if F == 0:
                while True:
                    b = rs(ip); ip += 1
                    if b:
                        F += b + 0xF
                        break
                    F += 0xFF
            for _ in range(F + 3):
                out.append(rs(ip)); ip += 1
            F = rs(ip); ip += 1
            if F > 0xF:
                pass
            else:
                d = (F >> 2) + (rs(ip) << 2) + mid
                start = len(out) - d
                ip += 1
                for k in range(3):
                    out.append(rd(start + k))
                for _ in range(F & 3):
                    out.append(rs(ip)); ip += 1
                F = rs(ip); ip += 1
                continue
        if F <= 0x1F:
            b3 = w_b3 if (F & 8) else 0
            ln = F & 7
            if ln == 0:
                while True:
                    b = rs(ip); ip += 1
                    if b:
                        ln += b + 7
                        break
                    ln += 0xFF
            p = ip
            v = (len(out) - b3 - (rs(p) >> 2) - (rs(p + 1) << 6)) & 0xFFFFFFFF
            ip += 2
            if v == len(out):
                if ln != 1:
                    raise LZErr(f'term len={ln}')
                if ip != src_len:
                    raise LZErr(f'ip {ip} != {src_len}')
                return bytes(out)
            start = (v - w_adj) & 0xFFFFFFFF
            if start >= 0x80000000:
                start -= 0x100000000
            for k in range(ln + 2):
                out.append(rd(start + k))
            cnt = {'b0': rs(p) & 3, 'tok': F & 3, 'zero': 0}[w_cnt]
        elif F <= 0x3F:
            ln = F - 0x20
            if ln == 0:
                while True:
                    b = rs(ip); ip += 1
                    if b:
                        ln += b + 0x1F
                        break
                    ln += 0xFF
            p = ip
            start = len(out) - (rs(p) >> 2) - (rs(p + 1) << 6) - 1
            ip += 2
            for k in range(ln + 2):
                out.append(rd(start + k))
            cnt = {'b0': rs(p) & 3, 'tok': F & 3, 'zero': 0}[a2_cnt]
        else:
            nib = {'r2m7': (F >> 2) & 7, 'm7': F & 7, 'r3m7': (F >> 3) & 7, 'm15': F & 15}[x_nib]
            ln = (F >> 5) - 1
            start = len(out) - nib - (rs(ip) << x_sh) - 1
            ip += 1
            for k in range(ln + 2):
                out.append(rd(start + k))
            cnt = {'tok': F & 3, 'b0': rs(ip - 1) & 3, 'zero': 0}[x_cnt]
        for _ in range(cnt):
            out.append(rs(ip)); ip += 1
        F = rs(ip); ip += 1


def load_block(app, gaddr, glen):
    d = open(app, 'rb').read()
    f = gaddr - 0x101000 + 0x20
    return d[f:f + glen]


def main():
    yz = load_block(r'雪鲤鱼模拟器-0.1/rom/宇宙风暴/yzfb_QVGA_SNOWFISHV2.app', 0x123493, 3974)
    midi = load_block(r'雪鲤鱼模拟器-0.1/rom/黄金矿工/hjkg_QVGA_SNOWFISHV2.app', 0x122B94, 2217)
    best = []
    for w_cnt, a2_cnt, x_cnt, w_adj, w_b3, mid, x_nib, x_sh in itertools.product(
        ('b0', 'tok', 'zero'), ('b0', 'tok', 'zero'), ('tok', 'b0', 'zero'),
        (0x4000, 0x1000, 0x400), (0x4000, 0x1000), (0x801, 0x800, 1),
        ('r2m7', 'm7', 'r3m7', 'm15'), (3, 2, 4),
    ):
        P = dict(w_cnt=w_cnt, a2_cnt=a2_cnt, x_cnt=x_cnt, w_adj=w_adj,
                 w_b3=w_b3, mid=mid, x_nib=x_nib, x_sh=x_sh)
        try:
            out = lz(yz, 3974, b'', P)
            yz_ok = len(out) == 16384
        except LZErr as e:
            yz_ok = False
            msg = str(e)
        try:
            mout = lz(midi, 2217, b'', P)
            midi_ok = mout[:22] == bytes.fromhex('00010ab94d546864000000060001000501e04d54726b')
            midi_len = len(mout)
        except LZErr:
            midi_ok = False
            midi_len = 0
        if yz_ok or midi_ok:
            best.append((yz_ok, midi_ok, midi_len, P))
            print(f"YZ={yz_ok} MIDI={midi_ok}({midi_len}) {P}")
    if not best:
        print("no combination satisfies the invariants")


if __name__ == '__main__':
    main()
