//! Horizontal layout of a layer: items in order, fixed or proportional widths, a gap
//! between neighbours. A pure function of the sizes, so it can be tested on its own.

/// How wide an item wants to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// Exactly this many pixels.
    Fixed(f32),
    /// A share of what the fixed items and gaps leave, proportional to the weight.
    Stretch(f32),
}

/// Places `sizes` left to right in `[x0, x0 + total)`, `gap` px apart. Returns each
/// item's (x, width), with edges on whole pixels so borders stay sharp.
///
/// Stretch items share the leftover space by weight; if there is none they get zero
/// width. If the fixed items alone don't fit, the row simply runs past the end: the
/// caller decides what to do with items that end beyond `x0 + total`.
pub fn distribute(x0: f32, total: f32, gap: f32, sizes: &[Size]) -> Vec<(f32, f32)> {
    let gaps = gap * sizes.len().saturating_sub(1) as f32;
    let fixed: f32 = sizes
        .iter()
        .map(|s| match s {
            Size::Fixed(w) => *w,
            Size::Stretch(_) => 0.0,
        })
        .sum();
    let weights: f32 = sizes
        .iter()
        .map(|s| match s {
            Size::Stretch(k) => *k,
            Size::Fixed(_) => 0.0,
        })
        .sum();
    let leftover = (total - fixed - gaps).max(0.0);

    // Accumulate exact positions and round each edge, so rounding errors don't add up
    // and the last stretch item ends exactly at the right edge.
    let mut out = Vec::with_capacity(sizes.len());
    let mut x = x0;
    for s in sizes {
        let w = match s {
            Size::Fixed(w) => *w,
            Size::Stretch(k) if weights > 0.0 => leftover * k / weights,
            Size::Stretch(_) => 0.0,
        };
        let (left, right) = (x.round(), (x + w).round());
        out.push((left, right - left));
        x += w + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Size::*;

    #[test]
    fn fixed_only() {
        let r = distribute(4.0, 1000.0, 10.0, &[Fixed(100.0), Fixed(50.0)]);
        assert_eq!(r, vec![(4.0, 100.0), (114.0, 50.0)]);
    }

    #[test]
    fn spacer_pushes_to_the_right() {
        // [80] [spacer] [100]: the last item ends at the right edge.
        let r = distribute(
            0.0,
            1000.0,
            10.0,
            &[Fixed(80.0), Stretch(1.0), Fixed(100.0)],
        );
        assert_eq!(r[0], (0.0, 80.0));
        assert_eq!(r[1], (90.0, 800.0));
        assert_eq!(r[2], (900.0, 100.0));
    }

    #[test]
    fn stretch_by_weight() {
        let r = distribute(0.0, 300.0, 0.0, &[Stretch(1.0), Stretch(2.0)]);
        assert_eq!(r, vec![(0.0, 100.0), (100.0, 200.0)]);
    }

    #[test]
    fn rounding_keeps_edges_contiguous() {
        let r = distribute(0.0, 100.0, 0.0, &[Stretch(1.0), Stretch(1.0), Stretch(1.0)]);
        assert_eq!(r[0].0, 0.0);
        for pair in r.windows(2) {
            assert_eq!(pair[0].0 + pair[0].1, pair[1].0);
        }
        let last = r[2];
        assert_eq!(last.0 + last.1, 100.0);
    }

    #[test]
    fn no_room_for_stretch() {
        let r = distribute(0.0, 100.0, 10.0, &[Fixed(80.0), Stretch(1.0), Fixed(80.0)]);
        assert_eq!(r[1].1, 0.0);
        // Overflows: the caller sees the last item ending past 100.
        assert!(r[2].0 + r[2].1 > 100.0);
    }

    #[test]
    fn empty() {
        assert!(distribute(0.0, 100.0, 10.0, &[]).is_empty());
    }
}
