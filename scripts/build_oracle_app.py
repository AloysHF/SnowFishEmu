"""Builds a synthetic .app that decompresses a target block and dumps the
output bytes as fill() colors — turning the reference emulator's verbose
trace into a byte-exact decompression oracle."""
import struct


class Asm:
    def __init__(self):
        self.code = bytearray()
        self.labels = {}
        self.fixups = []  # (offset, kind, label) kind: b/bl/lit
        self.relocs = []  # code offsets of words to relocate (+CODE_BASE)

    def emit(self, word):
        self.code.extend(struct.pack('<I', word))

    def label(self, name):
        while len(self.code) % 4:
            self.code.append(0)
        self.labels[name] = len(self.code)

    def ldr_lit(self, reg, label, relocate=False):
        """ldr reg,[pc,#off] with a forward literal placed by lit_word."""
        off = len(self.code)
        self.emit(0xE59F0000 | (reg << 12))
        self.fixups.append((off + 4, 'lit', (label, relocate)))

    def lit_word(self, label, value=None, relocate=False):
        while len(self.code) % 4:
            self.code.append(0)
        self.labels[label] = len(self.code)
        self.code.extend(struct.pack('<I', value or 0))
        if relocate:
            self.relocs.append(len(self.code) - 4)

    def b(self, label, cond=0xA):
        off = len(self.code)
        self.emit(cond << 28 | 0x0A000000)
        self.fixups.append((off, 'b', label))

    def bl(self, label):
        off = len(self.code)
        self.emit(0xEB000000)
        self.fixups.append((off, 'b', label))

    def resolve(self, code_base):
        for off, kind, label in self.fixups:
            target = self.labels[label]
            if kind == 'lit':
                # the word at `off` holds a literal value; for LDR the pc
                # relative offset must be patched instead — handled by
                # storing the literal's offset and patching both.
                pass
        # two-pass: literals record their offsets; patch LDR offsets and,
        # for relocated literals, the rotab entries reference the word.
        # Simpler: we stored literal words inline; patch LDR offsets here.
        # fixups with kind 'lit' store (ldr_off+4? no) — see ldr_lit: fixup
        # records the LDR instruction offset+4?? — rebuild: ldr_lit pushed
        # (off+4) but we need the LDR's own offset; store differently.
        raise NotImplementedError


class Builder:
    """Simpler two-pass builder: pass 1 lays out with placeholder zeros and
    records label positions; pass 2 patches branches and literal loads."""

    def __init__(self):
        self.code = bytearray()
        self.labels = {}
        self.branch_fix = []   # (off, label, cond_op)
        self.ldr_fix = []      # (off, reg, lit_label)
        self.lit_values = {}   # lit_label -> (value, relocate)
        self.lit_order = []

    def emit(self, word):
        self.code.extend(struct.pack('<I', word))

    def label(self, name):
        while len(self.code) % 4:
            self.code.append(0)
        self.labels[name] = len(self.code)

    def branch(self, label, op=0x0A000000):
        self.branch_fix.append((len(self.code), label, op))
        self.emit(op)

    def call(self, label):
        self.branch(label, 0x0B000000)

    def ldr_lit(self, reg, lit_label):
        self.ldr_fix.append((len(self.code), reg, lit_label))
        self.emit(0xE59F0000 | (reg << 12))

    def lit(self, name, value, relocate=False):
        """Places a 4-byte literal (call at the position you want it)."""
        while len(self.code) % 4:
            self.code.append(0)
        self.labels[name] = len(self.code)
        self.lit_values[name] = (value, relocate)
        self.code.extend(struct.pack('<I', value))

    def literal_space(self, name):
        self.label(name)

    def build(self):
        for off, label, op in self.branch_fix:
            target = self.labels[label]
            delta = (target - (off + 8)) >> 2
            struct.pack_into('<I', self.code, off, op | (delta & 0xFFFFFF))
        for off, reg, lit in self.ldr_fix:
            target = self.labels[lit]
            delta = target - (off + 8)
            assert 0 <= delta < 0x1000, (hex(off), hex(target))
            struct.pack_into('<I', self.code, off,
                             0xE59F0000 | (reg << 12) | (delta & 0xFFF))
        return bytes(self.code)

    def reloc_list(self):
        return [self.labels[name] for name, (v, rel) in self.lit_values.items()
                if rel]


def build(blocks):
    b = Builder()
    # --- entry: register the dispatch block ---
    b.label('entry')
    b.ldr_lit(0, 'lit_entry_block')
    b.emit(0xE5810000)      # str r0,[r1]  (block ptr -> args slot)
    b.emit(0xE12FFF1E)      # bx lr
    # --- dispatch ---
    b.label('dispatch')
    b.emit(0xE351000A)      # cmp r1,#10
    b.branch('dispatch_done', 0x1A000000)
    b.call('work')
    b.label('dispatch_done')
    b.emit(0xE12FFF1E)
    # --- work ---
    b.label('work')
    b.ldr_lit(0, 'lit_src')
    b.ldr_lit(1, 'lit_len')
    b.ldr_lit(2, 'lit_dst')
    b.emit(0xE24DD008)      # sub sp,sp,#8
    b.ldr_lit(12, 'lit_out')
    b.emit(0xE58DC000)      # str r12,[sp]
    b.emit(0xE1A0300C)      # mov r3,r12         (out ptr)
    b.call('lz_wrapper')
    b.emit(0xE59DC000)      # ldr r12,[sp]
    b.emit(0xE59C4000)      # ldr r4,[r12]
    b.ldr_lit(5, 'lit_dst')
    b.emit(0xE3A06000)      # mov r6,#0
    b.label('loop')
    b.emit(0xE1540006)      # cmp r6,r4
    b.branch('loop_end', 0xA000000)
    b.ldr_lit(0, 'lit_canvas')
    b.emit(0xE3A01000)      # mov r1,#0
    b.emit(0xE3A02000)      # mov r2,#0
    b.emit(0xE3A03000)      # mov r3,#1
    b.emit(0xE24DD008)      # sub sp,sp,#8
    b.emit(0xE3A0C001)      # mov r12,#1
    b.emit(0xE58DC000)      # str r12,[sp]        (h=1)
    b.emit(0xE5D5C000)      # ldrb r12,[r5,r6]
    b.emit(0xE58DC004)      # str r12,[sp,#4]     (color=byte)
    b.call('fill_wrapper')
    b.emit(0xE28DD008)      # add sp,sp,#8
    b.emit(0xE2866001)      # add r6,#1
    b.branch('loop')
    b.label('loop_end')
    b.emit(0xE28DD008)      # add sp,sp,#8
    b.emit(0xE12FFF1E)      # bx lr
    # --- lz wrapper ---
    b.label('lz_wrapper')
    b.emit(0xE52DE004)      # push {lr}
    b.ldr_lit(12, 'lit_c4')
    b.emit(0xE51CC004)      # ldr r12,[r12,#-4]  (hook)
    b.emit(0xE1A0E00F)      # mov lr,pc
    b.emit(0xE59CF108)      # ldr pc,[r12,#0x108]
    b.emit(0xE49DF004)      # pop {pc}           (ret = hook+0xC)
    # --- fill wrapper ---
    b.label('fill_wrapper')
    b.emit(0xE52DE004)      # push {lr}
    b.emit(0xE24DD008)      # sub sp,sp,#8       (adj=8)
    b.emit(0xE59DC00C)      # ldr r12,[sp,#0xC]  arg5
    b.emit(0xE58DC000)      # str r12,[sp]
    b.emit(0xE59DC010)      # ldr r12,[sp,#0x10] arg6
    b.emit(0xE58DC004)      # str r12,[sp,#4]
    b.ldr_lit(12, 'lit_c4')
    b.emit(0xE51CC004)      # hook
    b.emit(0xE1A0E00F)
    b.emit(0xE59CF12C)      # ldr pc,[r12,#0x12C]
    b.emit(0xE28DD008)      # add sp,#8          (ret = hook+0xC)
    b.emit(0xE49DF004)      # pop {pc}
    # --- dispatch block: [0] = dispatch fn address (relocated) ---
    b.label('block')
    b.lit('word_dispatch', 0, relocate=True)

    # --- literals ---
    b.lit('lit_entry_block', 0, relocate=True)
    b.lit('lit_c4', 0xC4, relocate=True)
    b.lit('lit_dst', 0x600024)
    b.lit('lit_out', 0x3FFF00)
    b.lit('lit_canvas', 0x540710)
    b.lit('lit_len', len(blocks[0]))
    b.lit('lit_src', 0, relocate=True)
    code_patched = b.build()
    data = bytearray(code_patched)
    while len(data) % 4:
        data.append(0)
    data_offsets = []
    for blk in blocks:
        data_offsets.append(len(data))
        data.extend(blk)
        while len(data) % 4:
            data.append(0)
    # The loader clobbers the LAST 8 bytes of the image (root/inject words);
    # keep the data blocks clear of them.
    data.extend(bytes(16))
    lit_src_off = b.labels['lit_src']
    struct.pack_into('<I', data, lit_src_off, data_offsets[0])  # rotab adds base
    # block[0] = dispatch OFFSET (relocation turns it into the address);
    # entry literal = block OFFSET (same relocation).
    block_off = b.labels['word_dispatch']
    struct.pack_into('<I', data, block_off, b.labels['dispatch'])
    entry_lit_off = b.labels['lit_entry_block']
    struct.pack_into('<I', data, entry_lit_off, b.labels['block'])
    relocs = b.reloc_list()  # lit_src already included (relocate=True)

    app = bytearray()
    app.extend(struct.pack('<I', 0x19414653))
    app.extend(struct.pack('<I', 0))
    app.extend(struct.pack('<I', 0))
    app.extend(struct.pack('<I', 0))
    app.extend(struct.pack('<I', 0x20))
    app.extend(struct.pack('<I', len(data)))
    app.extend(struct.pack('<I', 0))
    app.extend(struct.pack('<I', len(relocs)))
    app.extend(data)
    for r in relocs:
        app.extend(struct.pack('<I', r))
    meta = {'data_offsets': data_offsets}
    return bytes(app), meta


def main():
    d = open(r'雪鲤鱼模拟器-0.1/rom/宇宙风暴/yzfb_QVGA_SNOWFISHV2.app', 'rb').read()
    yz = d[0x123493 - 0x101000 + 0x20:][:3974]
    tiny = bytes([0x16, ord('A'), ord('B'), ord('C'), ord('D'), ord('E'),
                  0x4C, 0x00, 0x11, 0x00, 0x00])
    app, _ = build([tiny, yz])
    open('oracle.app', 'wb').write(app)
    print(f'oracle.app: {len(app)} bytes')


if __name__ == '__main__':
    main()
