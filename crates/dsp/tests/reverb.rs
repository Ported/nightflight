//! What a reverb must do, measured. Per-band decay times are checked from a
//! rendered WAV by `tools/analyse.py --rt60`, which has a proper band filter;
//! these are the properties worth failing a build over.

use dsp::SR;
use dsp::reverb::Reverb;

/// The impulse response of a reverb, both ears.
fn impulse(rt60: f32, damping: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let mut reverb = Reverb::new(SR, rt60, damping, 0.03, 100.0);
    let n = (seconds * SR) as usize;
    let mut bus = vec![0.0f32; n];
    bus[0] = 1.0;
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    reverb.process(&bus, &mut left, &mut right, 1.0);
    (left, right)
}

/// Decay time by Schroeder's method: integrate the squared response backwards,
/// then take the slope between -5 and -35 dB and extrapolate to -60. The last
/// 25 dB is left out because that is where the noise floor lives.
fn rt60_of(x: &[f32]) -> f32 {
    let mut decay = vec![0.0f64; x.len()];
    let mut sum = 0.0;
    for (i, &s) in x.iter().enumerate().rev() {
        sum += f64::from(s) * f64::from(s);
        decay[i] = sum;
    }
    let reference = decay[0];
    let at = |level: f64| {
        decay
            .iter()
            .position(|&d| 10.0 * (d / reference).log10() <= level)
            .unwrap_or(x.len() - 1)
    };
    let (a, b) = (at(-5.0), at(-35.0));
    let seconds = (b - a) as f32 / SR;
    let slope = -30.0 / seconds; // dB per second
    -60.0 / slope
}

/// Zero crossings per second: a cheap, robust measure of brightness.
fn brightness(x: &[f32]) -> f32 {
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    crossings as f32 / (x.len() as f32 / SR)
}

#[test]
fn undamped_it_decays_at_exactly_the_requested_rate() {
    // With no damping every frequency decays together, so the broadband figure
    // is the figure. This is what makes `rt60` a parameter and not a hint.
    for wanted in [1.5f32, 4.0, 7.0] {
        let (left, _) = impulse(wanted, 0.0, wanted * 1.4);
        let measured = rt60_of(&left);
        let error = (measured / wanted - 1.0).abs();
        assert!(
            error < 0.08,
            "asked for {wanted} s, measured {measured:.2} s ({:.0}% out)",
            error * 100.0
        );
    }
}

#[test]
fn damping_makes_the_tail_darken() {
    // A real room soaks up highs faster than lows, so the tail should get
    // duller as it dies. Brightness early against brightness late says so
    // without needing a spectrum.
    let (left, _) = impulse(7.0, 0.6, 8.0);
    let early = brightness(&left[(0.05 * SR) as usize..(0.4 * SR) as usize]);
    let late = brightness(&left[(6.0 * SR) as usize..(7.5 * SR) as usize]);
    assert!(
        late < 0.7 * early,
        "the tail did not darken: {early:.0} Hz early, {late:.0} Hz late"
    );

    // And with damping off it should not darken at all.
    let (flat, _) = impulse(7.0, 0.0, 8.0);
    let early = brightness(&flat[(0.05 * SR) as usize..(0.4 * SR) as usize]);
    let late = brightness(&flat[(6.0 * SR) as usize..(7.5 * SR) as usize]);
    assert!(
        (late / early - 1.0).abs() < 0.25,
        "undamped, the tail changed colour: {early:.0} Hz early, {late:.0} Hz late"
    );
}

#[test]
fn the_ears_hear_different_tails() {
    // What makes a tail surround you rather than sit inside your head. The
    // Python gets it by using different noise in each ear; a delay network has
    // to arrange it, by mixing the same lines into each ear with different
    // signs.
    let (left, right) = impulse(7.0, 0.6, 8.0);
    let n = left.len() as f32;
    let mean = |x: &[f32]| x.iter().sum::<f32>() / n;
    let (ml, mr) = (mean(&left), mean(&right));
    let cov: f32 = left
        .iter()
        .zip(&right)
        .map(|(l, r)| (l - ml) * (r - mr))
        .sum();
    let var = |x: &[f32], m: f32| x.iter().map(|s| (s - m) * (s - m)).sum::<f32>().sqrt();
    let correlation = cov / (var(&left, ml) * var(&right, mr));
    assert!(
        correlation.abs() < 0.2,
        "the ears are {correlation:+.3} correlated: the tail will sit in the middle of the head"
    );
}

#[test]
fn a_send_of_one_is_as_loud_as_the_dry_signal() {
    // The response is normalised to unit energy, so turning a source's send to
    // 1 makes its reverb as loud as itself. Without this, changing rt60 would
    // change the wet level and every send would need rebalancing.
    let (left, right) = impulse(7.0, 0.6, 9.0);
    let energy: f32 = left.iter().chain(&right).map(|s| s * s).sum::<f32>() / 2.0;
    assert!(
        (0.9..1.1).contains(&energy),
        "unit energy is what the sends assume; got {energy:.3}"
    );

    // And the same claim where it actually matters: fed continuously rather
    // than struck once. Unit energy in the response is what makes these two
    // the same statement, but only one of them is what a send does in a mix,
    // and a scaling error anywhere else would show up here and not above.
    for (rt60, damping) in [(1.5f32, 0.0f32), (3.5, 0.6), (7.0, 1.0)] {
        let mut reverb = Reverb::new(SR, rt60, damping, 0.03, 0.0);
        let n = (20.0 * SR) as usize;
        let mut state = 99u32;
        let mut input = vec![0.0f32; n];
        for s in &mut input {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *s = (state >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0;
        }
        let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
        reverb.process(&input, &mut left, &mut right, 1.0);
        // From halfway, so the tail has finished building.
        let from = n / 2;
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        let wet = 0.5 * (rms(&left[from..]) + rms(&right[from..]));
        let db = 20.0 * (wet / rms(&input[from..])).log10();
        assert!(
            db.abs() < 1.0,
            "rt60 {rt60}, damping {damping}: a send of 1 came back {db:+.2} dB"
        );
    }
}

#[test]
fn it_is_stable_and_goes_quiet() {
    // A feedback network that is not quite lossless rings forever or explodes.
    // Drive it hard for two seconds, then listen to the silence.
    let mut reverb = Reverb::new(SR, 4.0, 0.5, 0.03, 100.0);
    let mut state = 12_345u32;
    let mut noise = vec![0.0f32; (2.0 * SR) as usize];
    for s in &mut noise {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *s = (state >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0;
    }
    let mut left = vec![0.0f32; noise.len()];
    let mut right = vec![0.0f32; noise.len()];
    reverb.process(&noise, &mut left, &mut right, 1.0);
    let driven = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(driven.is_finite() && driven < 4.0, "ran away to {driven}");

    // Now feed silence for one and a half decay times.
    let silence = vec![0.0f32; (6.0 * SR) as usize];
    let mut tail_l = vec![0.0f32; silence.len()];
    let mut tail_r = vec![0.0f32; silence.len()];
    reverb.process(&silence, &mut tail_l, &mut tail_r, 1.0);
    let end = tail_l.len() - (0.2 * SR) as usize;
    let residue = tail_l[end..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let fallen = 20.0 * (residue / driven).log10();
    assert!(
        fallen < -60.0,
        "after 6 s of silence the tail is only {fallen:.1} dB down"
    );
    assert!(
        tail_l[..(0.1 * SR) as usize].iter().any(|s| s.abs() > 1e-6),
        "the tail stopped the instant the input did"
    );
}
