"""Builds single-token-class LZ test apps and extracts the oracle's
decoded byte count from the reference emulator's fill trace."""
import struct
import sys

sys.path.insert(0, r'D:\Code\Emulator\SnowFishEmu\tmp')
import build_oracle_app as B

TESTS = {
    # init 5 literals "ABCDE" then terminator
    'lit_only': bytes([0x16, 0x41, 0x42, 0x43, 0x44, 0x45, 0x11, 0x00, 0x00]),
    # + X token: F=0x4C ln=1 nib=3 ob=0 -> off = 5-3-0-1 = 1 -> copies BCD
    'x_copy': bytes([0x16, 0x41, 0x42, 0x43, 0x44, 0x45, 0x4C, 0x00, 0x11, 0x00, 0x00]),
    # + A2 token: F=0x20 ln=0->ext b0=0x01 -> ln=1+0x1F+0x01?? no: ln=0 ext: b=0x01 -> ln = 0x01+0x1F = 32; copy 34 from off=5-0-0-1=4?? over-runs fwd.
    #   use b0=0x01 b1=0x00: off = 5 - 0 - 0 - 1 = 4 -> copy ln+2... ext makes 32. too long.
    #   F=0x21: ln=1, b0=0x00 b1=0x00: off=5-0-0-1=4?? off must be <5. off=4 -> copy out[4..4+3)= 'E'+? fwd? out len 5, copy 3 from 4: E + fwd 2 -> invalid.
    #   pick F=0x21 b0=0x04 b1=0x00: off = 5-1-0-1 = 3 -> out[3..6)=D,E,+fwd -> invalid (only 5 bytes).
    #   simplest: copy 2 bytes needs ln=0 -> F=0x20 with ext... ext forced. So A2 min copy is 2 when b0>>2=0,b1=0 & ln=0 -> but ln=0 triggers ext path! ext: b0=0x01 -> ln=1+31=32. so min A2 copy = 2 only via ln>=... hmm ln=0 -> ext; ln>=1 -> copy>=3. test F=0x21 b0=0x00 b1=0x00 off=4 copy 3: out[4]=E, then fwd reads -> in-window? off+2=6 >5 -> fwd error.
    #   => A2 with off=2: b0 = (5-2-1)<<2 = 8 = 0x08, b1=0. F=0x21: copy out[2..5)=C,D,E.
    'a2_copy': bytes([0x16, 0x41, 0x42, 0x43, 0x44, 0x45, 0x21, 0x08, 0x00, 0x11, 0x00, 0x00]),
    # + W token: F=0x10 b3=0: ln=0 -> ext! F=0x11: ln=1, b0,b1: off = V-0x4000 where V=5-(b0>>2)-(b1<<6). want off=2 -> V=2-16384 negative... V must be >= 0x4000 for off>=0?? W is for LONG distances (>=0x4000 back). Can't test in an 8-byte output without the window. Skip W here.
    'x_copy2': bytes([0x16, 0x41, 0x42, 0x43, 0x44, 0x45, 0x4E, 0x04, 0x11, 0x00, 0x00]),
    # X F=0x4E: ln=(7)-1?? (0x4E>>5)=2 -> ln=1; nib=(0x13)&7=3; ob=0x04 -> off = 5-3-32-1 <0 invalid. skip.
    'a2_copy2': bytes([0x16, 0x41, 0x42, 0x43, 0x44, 0x45, 0x2A, 0x04, 0x00, 0x11, 0x00, 0x00]),
    # A2 F=0x2A: ln=10, b0=0x04 -> b0>>2=1; off = 5-1-0-1=3; copy 12 -> fwd error. skip.
}


def app_for(stream):
    return B.build([stream])[0]


def main():
    import os
    os.makedirs('cases', exist_ok=True)
    for name, stream in TESTS.items():
        app = app_for(stream)
        open(f'cases/{name}.app', 'wb').write(app)
    print('built', list(TESTS))


if __name__ == '__main__':
    main()
