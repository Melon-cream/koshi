"""Minimal ANSI terminal emulator used to assert on real client output.

Feed raw client bytes in, read the resulting screen as text. Enough to check
what a user would see: cursor addressing, erase, alt-screen switches, scrolling.
SGR keeps the foreground colour of every cell, so a colour-only signal can be
read back; every other attribute is ignored.
"""

import codecs


class Screen:
    def __init__(self, rows=24, cols=80):
        self.rows = rows
        self.cols = cols
        self.buf = [[" "] * cols for _ in range(rows)]
        # Foreground colour of each cell: None for the terminal default, an int
        # for a 256-colour index, an (r, g, b) tuple for a true colour.
        self.fg_grid: list[list] = []
        for _ in range(rows):
            self.fg_grid.append([None] * cols)
        # Reverse video per cell, which is how the client marks a selection.
        self.rev_grid: list[list] = []
        self.bg_grid: list[list] = []
        for _ in range(rows):
            self.rev_grid.append([False] * cols)
            self.bg_grid.append([None] * cols)
        self.fg = None
        self.bg = None
        self.reverse = False
        self.row = 0
        self.col = 0
        self.alt = False
        # A box character is three bytes; a read can split it, so decode
        # incrementally instead of replacing the halves with a broken glyph.
        self._decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self._pending = ""
        self._saved = None
        # OSC bodies (feeds like ]52;c;...) are recorded but never drawn.
        self.osc = []

    def _write_char(self, ch):
        w = 2 if ord(ch) > 0x2E7F else 1
        if self.col >= self.cols:
            self.col = 0
            self._line_feed()
        if 0 <= self.row < self.rows and 0 <= self.col < self.cols:
            self.buf[self.row][self.col] = ch
            self.fg_grid[self.row][self.col] = self.fg
            self.rev_grid[self.row][self.col] = self.reverse
            self.bg_grid[self.row][self.col] = self.bg
            for k in range(1, w):
                if self.col + k < self.cols:
                    self.buf[self.row][self.col + k] = ""
                    self.fg_grid[self.row][self.col + k] = self.fg
                    self.rev_grid[self.row][self.col + k] = self.reverse
                    self.bg_grid[self.row][self.col + k] = self.bg
        self.col += w

    def _line_feed(self):
        if self.row == self.rows - 1:
            self.buf.pop(0)
            self.buf.append([" "] * self.cols)
            self.fg_grid.pop(0)
            self.fg_grid.append([None] * self.cols)
            self.rev_grid.pop(0)
            self.rev_grid.append([False] * self.cols)
            self.bg_grid.pop(0)
            self.bg_grid.append([None] * self.cols)
        else:
            self.row += 1

    def _blank(self, row, start=0, end=None):
        end = self.cols if end is None else end
        for col in range(start, end):
            self.buf[row][col] = " "
            self.fg_grid[row][col] = None
            self.rev_grid[row][col] = False
            self.bg_grid[row][col] = None

    def _clear(self):
        self.buf = [[" "] * self.cols for _ in range(self.rows)]
        self.fg_grid = []
        self.rev_grid = []
        self.bg_grid = []
        for _ in range(self.rows):
            self.fg_grid.append([None] * self.cols)
            self.rev_grid.append([False] * self.cols)
            self.bg_grid.append([None] * self.cols)

    def _erase_display(self, mode):
        if mode in (2, 3):
            self._clear()
        elif mode == 0:
            self._blank(self.row, self.col)
            for row in range(self.row + 1, self.rows):
                self._blank(row)
        elif mode == 1:
            for row in range(0, self.row):
                self._blank(row)
            self._blank(self.row, 0, self.col + 1)

    def _erase_line(self, mode):
        if mode == 0:
            self._blank(self.row, self.col)
        elif mode == 1:
            self._blank(self.row, 0, self.col + 1)
        elif mode == 2:
            self._blank(self.row)

    def _sgr(self, params):
        parts = params.split(";") if params else [""]
        values = [int(part) if part.isdigit() else 0 for part in parts]
        index = 0
        while index < len(values):
            value = values[index]
            if value == 0:
                self.fg = None
                self.bg = None
                self.reverse = False
            elif value == 7:
                self.reverse = True
            elif value == 27:
                self.reverse = False
            elif value == 39:
                self.fg = None
            elif value == 49:
                self.bg = None
            elif value in (38, 48) and index + 1 < len(values):
                target = "fg" if value == 38 else "bg"
                kind = values[index + 1]
                if kind == 5 and index + 2 < len(values):
                    setattr(self, target, values[index + 2])
                    index += 2
                elif kind == 2 and index + 4 < len(values):
                    setattr(
                        self,
                        target,
                        (values[index + 2], values[index + 3], values[index + 4]),
                    )
                    index += 4
            elif 30 <= value <= 37:
                self.fg = value - 30
            elif 90 <= value <= 97:
                self.fg = value - 90 + 8
            elif 40 <= value <= 47:
                self.bg = value - 40
            elif 100 <= value <= 107:
                self.bg = value - 100 + 8
            index += 1

    def _csi(self, params, final):
        priv = params.startswith("?")
        if priv:
            params = params[1:]
        nums = []
        for part in params.split(";"):
            if part.isdigit():
                nums.append(int(part))
            else:
                nums.append(None)
        n1 = nums[0] if nums else None

        if priv:
            if final in ("h", "l") and n1 in (1047, 1049, 47):
                self.alt = final == "h"
                self._clear()
                self.row = 0
                self.col = 0
            return

        if final == "H" or final == "f":
            r = (nums[0] or 1) - 1 if nums else 0
            c = (nums[1] or 1) - 1 if len(nums) > 1 else 0
            self.row = max(0, min(self.rows - 1, r))
            self.col = max(0, min(self.cols - 1, c))
        elif final == "A":
            self.row = max(0, self.row - (n1 or 1))
        elif final == "B":
            self.row = min(self.rows - 1, self.row + (n1 or 1))
        elif final == "C":
            self.col = min(self.cols - 1, self.col + (n1 or 1))
        elif final == "D":
            self.col = max(0, self.col - (n1 or 1))
        elif final == "G":
            self.col = max(0, min(self.cols - 1, (n1 or 1) - 1))
        elif final == "d":
            self.row = max(0, min(self.rows - 1, (n1 or 1) - 1))
        elif final == "J":
            self._erase_display(n1 or 0)
        elif final == "K":
            self._erase_line(n1 or 0)
        elif final == "s":
            self._saved = (self.row, self.col)
        elif final == "u":
            if self._saved:
                self.row, self.col = self._saved
        elif final == "m":
            self._sgr(params)
        # anything else: ignore

    def feed(self, data):
        if isinstance(data, bytes):
            text = (self._pending + self._decoder.decode(data)).split("\x1b")
            self._pending = ""
        else:
            text = (self._pending + data).split("\x1b")
        # split() gives the leading text first, then escape sequences
        first = text[0]
        for ch in first:
            self._consume(ch)
        for seq in text[1:]:
            if not seq:
                self._pending = "\x1b"
                continue
            if seq[0] == "[":
                i = 1
                while i < len(seq) and not ("\x40" <= seq[i] <= "\x7e"):
                    i += 1
                if i >= len(seq):
                    self._pending = "\x1b" + seq
                    continue
                self._csi(seq[1:i], seq[i])
                rest = seq[i + 1:]
                for ch in rest:
                    self._consume(ch)
            elif seq[0] == "]":
                end = seq.find("\x07")
                if end < 0:
                    self._pending = "\x1b" + seq
                    continue
                self.osc.append(seq[1:end])
                for ch in seq[end + 1:]:
                    self._consume(ch)
            elif seq[0] in "78":
                rest = seq[2:] if len(seq) > 1 else ""
                for ch in rest:
                    self._consume(ch)
            else:
                rest = seq[1:]
                for ch in rest:
                    self._consume(ch)

    def _consume(self, ch):
        if ch == "\r":
            self.col = 0
        elif ch == "\n":
            self._line_feed()
        elif ch == "\b":
            self.col = max(0, self.col - 1)
        elif ch == "\t":
            self.col = min(self.cols - 1, (self.col // 8 + 1) * 8)
        elif ch < " ":
            pass
        else:
            self._write_char(ch)

    def text(self):
        return "\n".join("".join(cell for cell in row) for row in self.buf)

    def line(self, index):
        return "".join(self.buf[index]).rstrip()

    def fg_at(self, row, col):
        """Foreground colour written to one cell."""
        return self.fg_grid[row][col]

    def reverse_at(self, row, col):
        """True when the cell was drawn in reverse video."""
        return self.rev_grid[row][col]

    def bg_at(self, row, col):
        """Background colour written to one cell."""
        return self.bg_grid[row][col]

    def contains(self, needle):
        return needle in self.text()
