#!/usr/bin/env python3
"""Tests of scan.py's letter-row judgement (`row_fits`), runnable alone: `python3 -I tools/regress45/test_scan.py`.

The gravity training's choice (`str3:0x46`, `Choose the level of\\ngravity training\\n{num:1}x Gravity`): the game
types the level into the number slot and then the rest of the line a letter at a time, so the original shows `10`,
`10x`, `10xG`, ... under the overlay's paper. With the slot's number known (the overlay log's `nums=`), a row that is
the number followed by the start of what the line has after the slot is the page's own English; anything else is
judged as before.
"""
import sys
import tempfile
import types
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
try:                                    # scan.py draws with numpy and PIL; row_fits needs neither
    import numpy  # noqa: F401
    import PIL  # noqa: F401
except ImportError:                     # (`python3 -I` leaves out the user site-packages)
    sys.modules.setdefault('numpy', types.ModuleType('numpy'))
    pil = types.ModuleType('PIL')
    pil.Image = pil.ImageDraw = types.ModuleType('PIL.stub')
    sys.modules.setdefault('PIL', pil)

import scan  # noqa: E402

# `xtask text-dump` rows (key, address, length, lines, English)
DUMP = (
    'key\taddr\tlen\tlines\ttext\n'
    'str3:0x46\t07:BCDB\t25\t3\tChoose the level of\\ngravity training\\n         {num:1}x Gravity{auto}\n'
    'str3:0x99\t07:0000\t9\t1\t{name:1} got {num:2} Zenie\n'
)


class RowFits(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'en.tsv'
            path.write_text(DUMP, encoding='utf-8')
            cls.english = scan.load_english(path)
            cls.tails = scan.load_number_tails(path)
        cls.lines = cls.english['str3:0x46']
        cls.ten = scan.number_tails(cls.tails['str3:0x46'], ['10', '', ''])

    def fits(self, row):
        return scan.row_fits(row, self.lines, self.ten)

    def test_tails_of_the_page(self):
        self.assertEqual(self.tails['str3:0x46'], [(1, 'xGravity')])
        self.assertEqual(self.ten, [('10', 'xGravity')])
        # a name slot is not a number's: only `{num:2}` and what follows it up to the line's end
        self.assertEqual(self.tails['str3:0x99'], [(2, 'Zenie')])

    def test_the_number_then_the_line_being_typed(self):
        for row in ('10', '10x', '10xG', '10xGr', '10xGravity'):
            self.assertTrue(self.fits(row), row)

    def test_the_two_rows_the_old_rule_rejected(self):
        # the frames 4215-4222 of mg_grav_choice: without the slot's number these are judged another page's letters
        self.assertFalse(scan.row_fits('10x', self.lines))
        self.assertFalse(scan.row_fits('10xG', self.lines))

    def test_another_number(self):
        self.assertFalse(self.fits('20x'))
        self.assertFalse(self.fits('20xG'))

    def test_not_the_start_of_the_rest(self):
        self.assertFalse(self.fits('10xZ'))
        self.assertFalse(self.fits('10y'))
        self.assertFalse(scan.typed_after_number('10y', self.ten))
        self.assertFalse(scan.typed_after_number('10Gravity', self.ten))

    def test_half_the_number_then_the_line(self):
        self.assertFalse(self.fits('1x'))
        self.assertFalse(scan.typed_after_number('1xG', self.ten))

    def test_another_pages_letters(self):
        for row in ('Dropitem', 'Wishgranted', 'Itsasecret'):
            self.assertFalse(self.fits(row), row)

    def test_no_number_known(self):
        # an old xtask (no `nums=` in its log) or a slot the game left empty: judged as before
        self.assertFalse(scan.row_fits('10x', self.lines, scan.number_tails(self.tails['str3:0x46'], None)))
        self.assertFalse(scan.row_fits('10x', self.lines, scan.number_tails(self.tails['str3:0x46'], ['', '', ''])))
        self.assertEqual(scan.number_tails(self.tails['str3:0x46'], ['', '', '']), [])

    def test_the_old_rule_unchanged(self):
        # what the rule kept from before takes (most of the row is the line's letters in order: `1xG` 2 of 3) stays
        # taken with or without the number, though the new shape alone does not take it
        self.assertTrue(scan.row_fits('1xG', self.lines))
        self.assertTrue(self.fits('1xG'))
        self.assertTrue(self.fits('Choosethelevelof'))
        self.assertTrue(self.fits('gravitytrain'))


class ParseOvl(unittest.TestCase):
    HEAD = 'OVL wx=58 named=str3:0x46 tr=1 st=1 line=2 cell=10 base=05C8 lines=3 prog=0.82'
    TAIL = ' | shown=str3:0x46 paper=90,182,290,226 place=90,183,16 | behind=- paper=- place=- | letters=162:215:1;'

    def test_numbers(self):
        o = scan.parse_ovl(self.HEAD + ' nums=10,,' + self.TAIL)
        self.assertEqual(o['named']['nums'], ['10', '', ''])

    def test_old_log_without_numbers(self):
        self.assertIsNone(scan.parse_ovl(self.HEAD + self.TAIL)['named']['nums'])


if __name__ == '__main__':
    unittest.main()
