// Breakaway replay player. Replays a finished NHL game as if it were live and
// recomputes the win probability (web/wp.js) after every play.
(() => {
  "use strict";

  // Primary colours tuned to read on the dark background, plus an alternate
  // used when both teams' primaries would look alike.
  const COLORS = {
    ANA: ["#F47A38", "#B9975B"], ARI: ["#C8414B", "#E2D6B5"], BOS: ["#FFB81C", "#E8EEF6"], BUF: ["#4A8FE7", "#FCB514"],
    CGY: ["#E8313A", "#F1BE48"], CAR: ["#E03A3E", "#A2AAAD"], CHI: ["#E4002B", "#FFD100"], COL: ["#4F8FD6", "#B5546E"],
    CBJ: ["#4F7DC2", "#CE1126"], DAL: ["#00A86B", "#A2AAAD"], DET: ["#E31937", "#E8EEF6"], EDM: ["#FF6A1A", "#4F7DC2"],
    FLA: ["#E03A4A", "#B9975B"], LAK: ["#A2AAAD", "#E8EEF6"], MIN: ["#3BA55C", "#D94A55"], MTL: ["#E0313D", "#4F7DC2"],
    NSH: ["#FFB81C", "#4F7DC2"], NJD: ["#E23A3A", "#E8EEF6"], NYI: ["#F47D30", "#3C7FD6"], NYR: ["#4A7CE0", "#E03A4A"],
    OTT: ["#E04050", "#C69214"], PHI: ["#F74902", "#E8EEF6"], PIT: ["#FCB514", "#E8EEF6"], SJS: ["#00A3AD", "#E57200"],
    SEA: ["#99D9D9", "#E9072B"], STL: ["#4F7DC2", "#FCB514"], TBL: ["#4C7BD9", "#E8EEF6"], TOR: ["#3D74E0", "#E8EEF6"],
    UTA: ["#6CACE4", "#E8EEF6"], VAN: ["#00A35A", "#4F7DC2"], VGK: ["#B4975A", "#E8EEF6"], WSH: ["#E03A4A", "#4F7DC2"],
    WPG: ["#7FA6D9", "#AC162C"],
  };
  const FEED_KINDS = new Set(["G", "P", "PS", "PE", "END", "SOC"]);
  const SHOT_KINDS = new Set(["S", "G"]);
  const $ = (id) => document.getElementById(id);

  let model, report, index;
  let game = null, ev = [], samples = [], endSec = 3600;
  let t = 0, playing = false, speed = 60, lastTs = null, shownIdx = -1;

  // ---------------------------------------------------------------- utils
  const pct = (p) => `${Math.round(p * 100)}%`;
  const mmss = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  const season = (s) => `${String(s).slice(0, 4)}–${String(s).slice(6, 8)}`;
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const hexRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const dist = (a, b) => Math.hypot(...hexRgb(a).map((v, i) => v - hexRgb(b)[i]));

  function teamColors(home, away) {
    const h = (COLORS[home] || ["#4A8FE7"])[0];
    let a = (COLORS[away] || ["#E8EEF6"])[0];
    if (dist(h, a) < 110) a = (COLORS[away] || [, "#E8EEF6"])[1];
    if (dist(h, a) < 110) a = "#E8EEF6";
    return { h, a };
  }

  function clockAt(sec, period) {
    const g = game;
    if (sec < 3600 || (sec === 3600 && period <= 3)) {
      const p = Math.min(3, Math.floor(sec / 1200) + 1);
      return { label: ["1st", "2nd", "3rd"][p - 1], clock: mmss(p * 1200 - sec) };
    }
    if (g.type === 2) {
      if (period >= 5) return { label: "Shootout", clock: "SO" };
      return { label: "Overtime", clock: mmss(Math.max(0, 3900 - sec)) };
    }
    const n = Math.floor((sec - 3600) / 1200) + 1;
    return { label: n === 1 ? "Overtime" : `${n}OT`, clock: mmss(1200 - ((sec - 3600) % 1200)) };
  }

  // ---------------------------------------------------------------- model
  function wpAt(sec, st) {
    return WP.predict(model, sec, st.hs, st.as, st.sit, game.elo_diff, st.so, st.final);
  }

  function prepare(g) {
    game = g;
    const { h, a } = teamColors(g.home.abbrev, g.away.abbrev);
    document.documentElement.style.setProperty("--home", h);
    document.documentElement.style.setProperty("--away", a);

    let st = { hs: 0, as: 0, sit: "1551", so: false, final: false, period: 1 };
    let tie = 0, shotsH = 0, shotsA = 0;
    ev = [];
    samples = [[0, wpAt(0, st)]];
    for (const p of g.plays) {
      const [sec, period, kind, owner, sit, hs, as, note] = p;
      // sample the clock running down between events so the line drifts with time
      const from = samples[samples.length - 1][0];
      for (let s = Math.ceil((from + 1) / 15) * 15; s < sec; s += 15) samples.push([s, wpAt(s, st)]);
      const before = wpAt(sec, st);
      const so = g.type === 2 && period >= 5;
      const final = kind === "END" || kind === "SOC";
      if (!so && !final) tie = hs;
      const next = {
        hs: final ? g.final.home : so ? tie : hs,
        as: final ? g.final.away : so ? tie : as,
        sit: sit || st.sit, so, final, period,
      };
      if (SHOT_KINDS.has(kind) && !so) owner === 1 ? shotsH++ : owner === 0 ? shotsA++ : 0;
      // a goalie pull, but not the bookkeeping events after the game is decided
      const live = !final && !so && !["PE", "PS"].includes(kind) && !(sec >= 3600 && next.hs !== next.as);
      const pulled = !live ? -1 :
        (next.sit[3] === "0" && st.sit[3] !== "0") ? 1 :
        (next.sit[0] === "0" && st.sit[0] !== "0") ? 0 : -1;
      const after = wpAt(sec, next);
      ev.push({ sec, period, kind, owner, note, before, after, st: next, shotsH, shotsA, pulled });
      samples.push([sec, before], [sec, after]);
      st = next;
    }
    endSec = ev.length ? ev[ev.length - 1].sec : 3600;
    $("scrub").max = String(Math.ceil(endSec));
    $("homeAbbrev").textContent = g.home.abbrev;
    $("awayAbbrev").textContent = g.away.abbrev;
    $("homeName").textContent = g.home.name;
    $("awayName").textContent = g.away.name;
    $("homePctLab").textContent = `${g.home.abbrev} to win`;
    $("awayPctLab").textContent = `${g.away.abbrev} to win`;
    const meta = index.find((x) => String(x.id) === String(g.id));
    $("why").textContent = meta ? `${meta.why} · ${meta.label}` : "";
    t = 0; playing = false; shownIdx = -1;
    $("feed").innerHTML = "";
    setPlayIcon();
    render(true);
  }

  // ---------------------------------------------------------------- render
  function idxAt(time) {
    let lo = 0, hi = ev.length - 1, ans = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (ev[mid].sec <= time) { ans = mid; lo = mid + 1; } else hi = mid - 1;
    }
    return ans;
  }

  function teamOf(owner) { return owner === 1 ? game.home.abbrev : owner === 0 ? game.away.abbrev : ""; }

  function feedItem(e) {
    const c = clockAt(e.sec, e.period);
    const when = e.kind === "END" ? "" : `${c.label === "Shootout" ? "SO" : c.label.replace("Overtime", "OT")} ${c.clock === "SO" ? "" : c.clock}`;
    let cls = "", what = "";
    if (e.kind === "G") {
      cls = "goal";
      what = e.period >= 5 && game.type === 2
        ? `<b>${teamOf(e.owner)}</b> scores in the shootout`
        : `<b>${teamOf(e.owner)}</b> ${esc(e.note)} · ${e.st.as}–${e.st.hs}`;
    } else if (e.kind === "P") what = `<b>${teamOf(e.owner)}</b> penalty · ${esc(e.note)}`;
    else if (e.kind === "PS") { cls = "marker"; what = e.period === 1 ? "Puck drop" : `Start of ${clockAt(e.sec + 1, e.period).label}`; }
    else if (e.kind === "PE") { cls = "marker"; what = `End of ${clockAt(Math.max(0, e.sec - 1), e.period).label}`; }
    else if (e.kind === "END" || e.kind === "SOC") {
      cls = "marker";
      const w = game.final.home > game.final.away ? game.home.abbrev : game.away.abbrev;
      what = `Final · ${w} wins ${Math.max(game.final.home, game.final.away)}–${Math.min(game.final.home, game.final.away)}${game.final.last !== "REG" ? " (" + game.final.last + ")" : ""}`;
    }
    const d = e.after - e.before;
    const swing = Math.abs(d) >= 0.005 && cls !== "marker"
      ? `<span class="d" style="color:${d > 0 ? "var(--home)" : "var(--away)"}">+${(Math.abs(d) * 100).toFixed(1)}% ${d > 0 ? game.home.abbrev : game.away.abbrev}</span>`
      : "<span></span>";
    return `<li class="${cls}"><span class="t">${when}</span><span class="what">${what}</span>${swing}</li>`;
  }

  function pulledItem(e) {
    const team = e.pulled === 1 ? game.home.abbrev : game.away.abbrev;
    const c = clockAt(e.sec, e.period);
    return `<li><span class="t">${c.label.replace("Overtime", "OT")} ${c.clock}</span><span class="what"><b>${team}</b> pulls the goalie for an extra attacker</span><span></span></li>`;
  }

  function updateFeed(idx, animate) {
    const feed = $("feed");
    if (idx < shownIdx) { feed.innerHTML = ""; shownIdx = -1; }
    let html = "";
    for (let i = shownIdx + 1; i <= idx; i++) {
      const e = ev[i];
      if (e.pulled >= 0) html = pulledItem(e) + html;
      if (FEED_KINDS.has(e.kind)) html = feedItem(e) + html;
      if (animate && e.kind === "G" && !(e.period >= 5 && game.type === 2)) goalFlash(e);
    }
    if (html) {
      feed.insertAdjacentHTML("afterbegin", html);
      while (feed.children.length > 80) feed.lastElementChild.remove();
    }
    shownIdx = idx;
  }

  let flashTimer = 0;
  function goalFlash(e) {
    const box = e.owner === 1 ? $("homeBox") : $("awayBox");
    box.classList.add("is-goal");
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => { $("homeBox").classList.remove("is-goal"); $("awayBox").classList.remove("is-goal"); }, 2200);
    const light = $("goalLight");
    light.classList.remove("on"); void light.offsetWidth; light.classList.add("on");
    $("announce").textContent = `Goal ${teamOf(e.owner)}. ${e.note}. ${game.away.abbrev} ${e.st.as}, ${game.home.abbrev} ${e.st.hs}.`;
  }

  function strengthLabel(st) {
    const [ag, ask, hsk, hg] = WP.parseSituation(st.sit);
    const el = $("strength");
    el.className = "strength";
    if (st.final) return "Final";
    if (st.so) return "Shootout";
    if (hg === 0) { el.classList.add("is-en"); return `${game.home.abbrev} net empty`; }
    if (ag === 0) { el.classList.add("is-en"); return `${game.away.abbrev} net empty`; }
    if (hsk !== ask) { el.classList.add("is-pp"); return `Power play · ${hsk > ask ? game.home.abbrev : game.away.abbrev}`; }
    return `${ask} on ${hsk}`;
  }

  function render(rebuild = false, animate = false) {
    if (!game) return;
    const idx = idxAt(t);
    if (rebuild) { $("feed").innerHTML = ""; shownIdx = -1; }
    updateFeed(idx, animate);
    const e = idx >= 0 ? ev[idx] : null;
    const st = e ? e.st : { hs: 0, as: 0, sit: "1551", so: false, final: false, period: 1 };
    const p = st.final ? (game.final.home > game.final.away ? 1 : 0) : wpAt(t, st);

    $("homeScore").textContent = st.hs;
    $("awayScore").textContent = st.as;
    $("homeSog").textContent = e ? e.shotsH : 0;
    $("awaySog").textContent = e ? e.shotsA : 0;
    if (st.final) {
      $("period").textContent = "Final" + (game.final.last !== "REG" ? ` / ${game.final.last}` : "");
      $("clock").textContent = `${game.final.away}–${game.final.home}`;
    } else {
      const c = clockAt(t, st.period);
      $("period").textContent = c.label;
      $("clock").textContent = c.clock;
    }
    $("strength").textContent = strengthLabel(st);
    $("homePct").textContent = pct(p);
    $("awayPct").textContent = pct(1 - p);
    $("oddsFill").style.width = `${p * 100}%`;
    $("scrub").value = String(Math.round(t));
    drawChart(p, idx);
    drawSwings(idx);
  }

  // ---------------------------------------------------------------- chart
  function drawChart(pNow, idx) {
    const svg = $("chart");
    const W = svg.clientWidth, H = svg.clientHeight;
    if (!W || !H) return;
    const small = W < 520;
    const pl = small ? 8 : 12, pr = small ? 40 : 48, pt = 12, pb = 24;
    // the axis only grows into overtime once the replay gets there (no spoilers)
    const xMax = t <= 3600 ? 3600 : game.type === 2 ? 3900 : 3600 + 1200 * Math.ceil((t - 3600) / 1200);
    const x = (s) => pl + (Math.min(s, xMax) / xMax) * (W - pl - pr);
    const y = (p) => pt + (1 - p) * (H - pt - pb);
    const mid = y(0.5);

    const pts = [];
    for (const s of samples) { if (s[0] > t) break; pts.push(s); }
    pts.push([t, pNow]);
    const line = pts.map((s, i) => `${i ? "L" : "M"}${x(s[0]).toFixed(1)},${y(s[1]).toFixed(1)}`).join("");
    const area = `${line}L${x(t).toFixed(1)},${mid}L${x(0)},${mid}Z`;

    let g = `<defs><linearGradient id="fillg" gradientUnits="userSpaceOnUse" x1="0" y1="${y(1)}" x2="0" y2="${y(0)}">
      <stop offset="0" stop-color="var(--home)" stop-opacity=".42"/><stop offset=".5" stop-color="var(--home)" stop-opacity=".04"/>
      <stop offset=".5" stop-color="var(--away)" stop-opacity=".04"/><stop offset="1" stop-color="var(--away)" stop-opacity=".42"/>
    </linearGradient></defs>`;
    for (const p of [0.25, 0.75]) g += `<line class="c-grid" x1="${x(0)}" x2="${x(xMax)}" y1="${y(p)}" y2="${y(p)}"/>`;
    g += `<line class="c-mid" x1="${x(0)}" x2="${x(xMax)}" y1="${mid}" y2="${mid}"/>`;
    const bounds = [0, 1200, 2400, 3600];
    if (xMax > 3600) for (let s = 3600 + (game.type === 2 ? 300 : 1200); s <= xMax; s += 1200) bounds.push(s);
    bounds.push(xMax);
    const names = ["1st", "2nd", "3rd", "OT", "2OT", "3OT", "4OT"];
    for (let i = 0; i < bounds.length - 1; i++) {
      if (bounds[i] > 0) g += `<line class="c-grid" x1="${x(bounds[i])}" x2="${x(bounds[i])}" y1="${y(1)}" y2="${y(0)}"/>`;
      if (bounds[i + 1] > bounds[i]) g += `<text class="c-axis" x="${(x(bounds[i]) + x(bounds[i + 1])) / 2}" y="${H - 6}" text-anchor="middle">${names[i]}</text>`;
    }
    g += `<text class="c-team" x="${W - pr + 8}" y="${y(1) + 10}" fill="var(--home)">${game.home.abbrev}</text>`;
    g += `<text class="c-team" x="${W - pr + 8}" y="${y(0) - 2}" fill="var(--away)">${game.away.abbrev}</text>`;
    g += `<text class="c-axis" x="${W - pr + 8}" y="${mid + 4}">50%</text>`;
    g += `<path d="${area}" fill="url(#fillg)"/>`;
    for (let i = 0; i <= idx; i++) {
      const e = ev[i];
      if (e.kind === "P") g += `<line class="c-pen" x1="${x(e.sec)}" x2="${x(e.sec)}" y1="${y(0) - 6}" y2="${y(0)}"/>`;
    }
    g += `<path class="c-line" d="${line}"/>`;
    for (let i = 0; i <= idx; i++) {
      const e = ev[i];
      if (e.kind !== "G" || (e.period >= 5 && game.type === 2)) continue;
      const cx = x(e.sec), cy = y(e.after);
      const above = e.after < 0.5;
      g += `<circle class="c-goal" cx="${cx}" cy="${cy}" r="6.5" fill="${e.owner === 1 ? "var(--home)" : "var(--away)"}"/>`;
      if (!small) g += `<text class="c-goal-lab" x="${cx}" y="${above ? cy - 12 : cy + 20}" text-anchor="middle">${e.st.as}–${e.st.hs}</text>`;
    }
    const hx = x(t), hy = y(pNow);
    if (playing) g += `<circle class="c-pulse" cx="${hx}" cy="${hy}" r="5"/>`;
    g += `<circle class="c-head" cx="${hx}" cy="${hy}" r="5"/>`;
    svg.innerHTML = g;
    svg.setAttribute("aria-label",
      `Win probability: ${game.home.abbrev} ${pct(pNow)}, ${game.away.abbrev} ${pct(1 - pNow)}.`);
  }

  function drawSwings(idx) {
    const items = [];
    for (let i = 0; i <= idx; i++) {
      const e = ev[i];
      const d = e.after - e.before;
      if (Math.abs(d) >= 0.02 && !e.st.final) items.push({ e, d });
    }
    items.sort((a, b) => Math.abs(b.d) - Math.abs(a.d));
    const top = items.slice(0, 3);
    const el = $("swings");
    if (!top.length) {
      el.innerHTML = `<li class="empty">${idx < 0 ? "Press play. The biggest moments show up here as they happen." : "No big swings yet."}</li>`;
      return;
    }
    el.innerHTML = top.map(({ e, d }) => {
      const team = d > 0 ? game.home.abbrev : game.away.abbrev;
      const c = clockAt(e.sec, e.period);
      const what = e.kind === "G" ? `Goal · ${esc(e.note)}` : e.kind === "P" ? `Penalty · ${esc(e.note)}` : e.pulled >= 0 ? "Goalie pulled" : "Play";
      return `<li style="--c:${d > 0 ? "var(--home)" : "var(--away)"}"><div class="big">+${(Math.abs(d) * 100).toFixed(1)}% ${team}</div>
        <div class="sub">${c.label.replace("Overtime", "OT")} ${c.clock === "SO" ? "" : c.clock} · ${what}</div></li>`;
    }).join("");
  }

  // ---------------------------------------------------------------- model card
  function drawModelCard() {
    if (!report) return;
    const m = report.model, pre = report.baseline_pregame_rating_only;
    const cp = report.checkpoints || {};
    const rows = [
      ["Tested on", `${report.test_games.toLocaleString()} <small>games, ${season(report.test_season)}</small>`],
      ["Brier score (lower is better)", `${m.brier.toFixed(3)} <small>vs ${pre.brier.toFixed(3)} from pregame ratings alone</small>`],
      ["Favourite won, 5 min left", cp["5 min left"] ? `${Math.round(cp["5 min left"].favourite_won * 100)}%` : "—"],
      ["Favourite won, after the 1st", cp["after 1st"] ? `${Math.round(cp["after 1st"].favourite_won * 100)}%` : "—"],
    ];
    $("metrics").innerHTML = rows.map(([k, v]) => `<div><dt>${k}</dt><dd>${v}</dd></div>`).join("");
    const S = 220, P = 36;
    const sx = (v) => P + v * (S - P - 8), sy = (v) => S - P - v * (S - P - 8);
    let g = `<rect x="${sx(0)}" y="${sy(1)}" width="${sx(1) - sx(0)}" height="${sy(0) - sy(1)}" fill="none" stroke="#1F2A44"/>`;
    g += `<line x1="${sx(0)}" y1="${sy(0)}" x2="${sx(1)}" y2="${sy(1)}" stroke="#3A4766" stroke-dasharray="3 4"/>`;
    for (const v of [0, 0.5, 1]) {
      g += `<text x="${sx(v)}" y="${S - 8}" fill="#56627C" font-size="10" text-anchor="middle">${v * 100}%</text>`;
      g += `<text x="${P - 6}" y="${sy(v) + 3}" fill="#56627C" font-size="10" text-anchor="end">${v * 100}%</text>`;
    }
    const pts = report.calibration.map((b) => [sx(b.predicted), sy(b.actual), b.share_of_game_time]);
    g += `<path d="${pts.map((q, i) => `${i ? "L" : "M"}${q[0]},${q[1]}`).join("")}" fill="none" stroke="#4A8FE7" stroke-width="1.6"/>`;
    for (const q of pts) g += `<circle cx="${q[0]}" cy="${q[1]}" r="${3 + Math.sqrt(q[2]) * 10}" fill="#4A8FE7" fill-opacity=".85" stroke="#0F1628" stroke-width="1.5"/>`;
    $("calib").innerHTML = g;
  }

  // ---------------------------------------------------------------- playback
  function setPlayIcon() {
    $("playIcon").setAttribute("d", playing ? "M7 5h4v14H7zM13 5h4v14h-4z" : "M8 5v14l11-7z");
    $("playBtn").setAttribute("aria-label", playing ? "Pause" : "Play");
  }
  function play() { if (t >= endSec) { t = 0; render(true); } playing = true; lastTs = null; setPlayIcon(); }
  function pause() { playing = false; setPlayIcon(); render(); }

  function frame(ts) {
    if (playing) {
      if (lastTs !== null) {
        t = Math.min(endSec, t + ((ts - lastTs) / 1000) * speed);
        if (t >= endSec) { playing = false; setPlayIcon(); }
        render(false, true);
      }
      lastTs = ts;
    } else lastTs = null;
    requestAnimationFrame(frame);
  }

  async function loadGame(id) {
    pause();
    const g = await fetch(`games/${id}.json`).then((r) => r.json());
    prepare(g);
    $("gamePicker").value = String(id);
  }

  function bind() {
    $("playBtn").addEventListener("click", () => (playing ? pause() : play()));
    $("scrub").addEventListener("input", (e) => { t = +e.target.value; render(); });
    document.querySelectorAll(".speeds button").forEach((b) =>
      b.addEventListener("click", () => {
        speed = +b.dataset.speed;
        document.querySelectorAll(".speeds button").forEach((x) => x.classList.toggle("on", x === b));
      }));
    $("nextGoal").addEventListener("click", () => {
      const next = ev.find((e) => e.kind === "G" && e.sec > t + 1);
      if (!next) return;
      t = Math.max(0, next.sec - 10);
      render();
      play();
    });
    $("gamePicker").addEventListener("change", (e) => {
      try { history.replaceState(null, "", `#${e.target.value}`); } catch (_) { /* sandboxed viewers */ }
      loadGame(e.target.value);
    });
    document.addEventListener("keydown", (e) => {
      if (e.target.closest("input, select, textarea, button")) return;
      if (e.code === "Space") { e.preventDefault(); playing ? pause() : play(); }
    });
    const ro = new ResizeObserver(() => render());
    ro.observe($("fig"));
  }

  async function init() {
    [model, report, index] = await Promise.all([
      fetch("wp_model.json").then((r) => r.json()),
      fetch("report.json").then((r) => r.json()).catch(() => null),
      fetch("games/index.json").then((r) => r.json()),
    ]);
    $("gamePicker").innerHTML = index.map((g) => `<option value="${g.id}">${esc(g.label)} · ${esc(g.why)}</option>`).join("");
    bind();
    drawModelCard();
    const fromHash = location.hash.slice(1);
    const first = index.find((g) => String(g.id) === fromHash) || index[0];
    await loadGame(first.id);
    requestAnimationFrame(frame);
  }

  init().catch((err) => {
    $("why").textContent = "Couldn't load the replay data.";
    console.error(err);
  });
})();
