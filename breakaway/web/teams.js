// web/teams.js: team colours shared by the replay and trade pages.
(function (root) {
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
  const hexRgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const dist = (a, b) => Math.hypot(...hexRgb(a).map((v, i) => v - hexRgb(b)[i]));

  function teamColors(home, away) {
    const h = (COLORS[home] || ["#4A8FE7"])[0];
    let a = (COLORS[away] || ["#E8EEF6"])[0];
    if (dist(h, a) < 110) a = (COLORS[away] || [, "#E8EEF6"])[1];
    if (dist(h, a) < 110) a = "#E8EEF6";
    return { h, a };
  }

  root.BreakawayTeams = { COLORS, teamColors };
})(window);
