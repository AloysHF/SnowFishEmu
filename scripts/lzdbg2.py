"""Parameterized SnowFish LZ analysis with MIDI structural validation."""
import sys

class LZErr(Exception):
    pass


def lz(src, src_len, window=b'', P=None):
    P = P or {}
    out = bytearray()
    prev = bytes(window)
    prev_len = len(prev)

    def rd(i):
        if i < 0:
            j = prev_len + i
            if j < 0:
                return 0
            return prev[j] if j < prev_len else 0
        if i >= len(out):
            raise LZErr(f'fwd off={i} out={len(out)} ip={ip}')
        return out[i]

    def rs(i):
        if i >= src_len:
            raise LZErr(f'over ip={i}/{src_len} out={len(out)}')
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
                d = (F >> 2) + (rs(ip) << 2) + 0x801
                start = len(out) - d
                ip += 1
                for k in range(3):
                    out.append(rd(start + k))
                for _ in range(F & 3):
                    out.append(rs(ip)); ip += 1
                F = rs(ip); ip += 1
                continue
        if F <= 0x1F:
            b3 = 0x4000 if (F & 8) else 0
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
                    raise LZErr(f'term len={ln} ip={ip}')
                if ip != src_len:
                    raise LZErr(f'term ip={ip}!={src_len}')
                return bytes(out)
            start = v - 0x4000
            if start >= 0x80000000:
                start -= 0x100000000
            for k in range(ln + 2):
                out.append(rd(start + k))
            cnt = rs(p) & 3
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
            cnt = rs(p) & 3
        else:
            nib = (F >> 2) & 7
            ln = (F >> 5) - 1
            start = len(out) - nib - (rs(ip) << 3) - 1
            ip += 1
            for k in range(ln + 2):
                out.append(rd(start + k))
            cnt = F & 3
        for _ in range(cnt):
            out.append(rs(ip)); ip += 1
        F = rs(ip); ip += 1


def validate_midi(buf):
    """Parses buf as a MIDI file; returns (ok, pos, reason)."""
    n = len(buf)
    if n < 14 or buf[4:8] != b'MThd':
        return False, 4, 'no MThd'
    hlen = int.from_bytes(buf[8:12], 'big')
    i = 12 + hlen
    while True:
        if i + 8 > n:
            return True, i, 'end-of-buffer'
        if buf[i:i + 4] != b'MTrk':
            return False, i, f'no MTrk at {i}: {buf[i:i+8].hex()}'
        tlen = int.from_bytes(buf[i + 4:i + 8], 'big')
        body = i + 8
        end = body + tlen
        if end > n:
            return False, i, f'track {tlen} exceeds buffer ({n})'
        # strict event walk
        j = body
        running = None
        while j < end:
            while j < end and buf[j] & 0x80:
                j += 1  # delta continuation byte
            j += 1  # delta final byte
            if j >= end:
                return False, j, 'delta overruns track'
            st = buf[j]
            if st & 0x80:
                j += 1
                running = st
            else:
                st = running
                if st is None:
                    return False, j, 'running status without status'
            if st == 0xFF:
                j += 1
                mlen = 0
                while True:
                    if j >= end:
                        return False, j, 'meta len overruns'
                    b = buf[j]; j += 1
                    mlen = (mlen << 7) | (b & 0x7F)
                    if not (b & 0x80):
                        break
                j += mlen
                if j > end:
                    return False, j, 'meta data overruns track'
            elif st in (0xF0, 0xF7):
                return False, j - 1, 'sysex in a non-sysex file'
            elif (st & 0xF0) in (0x80, 0x90, 0xA0, 0xB0, 0xE0):
                j += 2
            elif (st & 0xF0) in (0xC0, 0xD0):
                j += 1
            else:
                return False, j - 1, f'bad status {st:#x}'
            if j > end:
                return False, j, 'event overruns track'
        i = end


def main():
    d = open(r'雪鲤鱼模拟器-0.1/rom/黄金矿工/hjkg_QVGA_SNOWFISHV2.app', 'rb').read()
    f = 0x122B94 - 0x101000 + 0x20
    src = d[f:f + 2217]
    out, how = lz(src, 2217)
    print(f'decoded {len(out)} bytes ({how})')
    ok, pos, why = validate_midi(out)
    print(f'midi validation: ok={ok} pos={pos} why={why}')
    print(f'context: {out[max(0,pos-8):pos+8].hex(" ")}')


if __name__ == '__main__':
    main()
