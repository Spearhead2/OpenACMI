"""Builds assets/stars.bin from the Yale Bright Star Catalogue.

    python tools/build_stars.py bsc5.dat.gz assets/stars.bin

The catalogue (5th revised edition, Hoffleit & Warren 1991, public domain)
lists the 9,110 objects brighter than about magnitude 6.5: the stars seen
with the naked eye. Get `bsc5.dat.gz` from
http://tdc-www.harvard.edu/catalogs/bsc5.html

Output, little-endian: the bytes `OASTARS1`, a u32 count, then per star
  u16  right ascension, J2000   0..65536 -> 0..360 degrees
  i16  declination, J2000       -32767..32767 -> -90..90 degrees
  i16  visual magnitude         in thousandths
  i16  B-V colour index         in thousandths
"""

import gzip
import struct
import sys


def main(source, target):
    opener = gzip.open if source.endswith(".gz") else open
    stars = []
    with opener(source, "rt", encoding="ascii", errors="replace") as lines:
        for line in lines:
            # Fixed columns. A few entries (novae, clusters) have no position.
            if len(line) < 107 or not line[75:83].strip() or not line[102:107].strip():
                continue
            ra = (int(line[75:77]) + int(line[77:79]) / 60 + float(line[79:83]) / 3600) * 15
            dec = int(line[84:86]) + int(line[86:88]) / 60 + int(line[88:90]) / 3600
            if line[83] == "-":
                dec = -dec
            magnitude = float(line[102:107])
            colour = float(line[109:114]) if line[109:114].strip() else 0.6
            stars.append((ra, dec, magnitude, colour))

    # Brightest first, so a reader can stop early at a magnitude limit.
    stars.sort(key=lambda star: star[2])
    with open(target, "wb") as out:
        out.write(b"OASTARS1")
        out.write(struct.pack("<I", len(stars)))
        for ra, dec, magnitude, colour in stars:
            out.write(struct.pack(
                "<Hhhh",
                round(ra / 360 * 65536) % 65536,
                round(dec / 90 * 32767),
                round(magnitude * 1000),
                max(-32768, min(32767, round(colour * 1000))),
            ))
    print(f"{len(stars)} stars, brightest {stars[0][2]:+.2f} at RA {stars[0][0]:.3f} Dec {stars[0][1]:.3f}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
