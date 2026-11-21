// web/wp.js: the browser twin of model/wp.py. Keep the two in lockstep;
// tests/test_parity.py checks they agree to 1e-9 on thousands of states.
(function (root) {
  "use strict";
  const REG_SECONDS = 3600;

  function parseSituation(code) {
    if (!code || code.length !== 4 || !/^\d{4}$/.test(code)) return [1, 5, 5, 1];
    return [+code[0], +code[1], +code[2], +code[3]];
  }

  function regFeatures(sec, home, away, situation, eloDiff) {
    const minsLeft = Math.max(0, (REG_SECONDS - sec) / 60);
    const lead = home - away;
    const [ag, ask, hsk, hg] = parseSituation(situation);
    let pp = hsk - ask;
    if (hg === 0) pp -= 1;
    if (ag === 0) pp += 1;
    // empty net only counts while the pulling team is behind (see model/wp.py)
    let pulled = 0;
    if (ag === 0 && lead > 0) pulled += 1;
    if (hg === 0 && lead < 0) pulled -= 1;
    const root = Math.sqrt(minsLeft + 1);
    const strength = eloDiff / 100;
    return [
      lead,
      lead / root,
      lead / (minsLeft + 1),
      strength,
      strength * (minsLeft / 60),
      pp,
      pp / root,
      pulled / root,
    ];
  }

  function otFeatures(situation, eloDiff) {
    const [, ask, hsk] = parseSituation(situation);
    return [eloDiff / 100, hsk - ask];
  }

  function sigmoid(z) {
    if (z >= 0) return 1 / (1 + Math.exp(-z));
    const e = Math.exp(z);
    return e / (1 + e);
  }

  function linear(m, x) {
    let z = m.intercept;
    for (let i = 0; i < x.length; i++) z += m.coef[i] * x[i];
    return z;
  }

  /** P(home team wins) for a game state. */
  function predict(model, sec, home, away, situation, eloDiff, shootout, final) {
    if (final) return home > away ? 1 : away > home ? 0 : 0.5;
    if (sec < REG_SECONDS) return sigmoid(linear(model.regulation, regFeatures(sec, home, away, situation, eloDiff)));
    if (home !== away) return home > away ? 1 : 0;
    if (shootout) return sigmoid(linear(model.shootout, [eloDiff / 100, 0]));
    return sigmoid(linear(model.overtime, otFeatures(situation, eloDiff)));
  }

  const api = { predict, regFeatures, otFeatures, parseSituation, REG_SECONDS };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.WP = api;
})(typeof window !== "undefined" ? window : globalThis);
