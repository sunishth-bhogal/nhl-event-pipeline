"""model/wp.py and web/wp.js must give identical probabilities."""
import json, pathlib, random, shutil, subprocess, sys, tempfile
HERE = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE / "model"))
from wp import predict

def test_parity():
    rng = random.Random(3)
    model = {k: {"intercept": rng.uniform(-1, 1), "coef": [rng.uniform(-2, 2) for _ in range(n)]}
             for k, n in (("regulation", 8), ("overtime", 2), ("shootout", 2))}
    sits = ["1551", "1451", "1541", "0651", "1560", "1331", "1441", "", "x"]
    cases = []
    for _ in range(5000):
        sec = rng.choice([0, 1, 59, 1200, 2399, 3540, 3599.5, 3600, 3700, 3900, rng.uniform(0, 3900)])
        cases.append([sec, rng.randint(0, 6), rng.randint(0, 6), rng.choice(sits),
                      rng.uniform(-200, 200), rng.random() < 0.1, rng.random() < 0.05])
    py = [predict(model, *c[:5], shootout=c[5], final=c[6]) for c in cases]
    # load as CommonJS regardless of the repo's "type": "module"
    cjs = pathlib.Path(tempfile.mkdtemp()) / "wp.cjs"
    shutil.copy(HERE / "web" / "wp.js", cjs)
    js_src = f"""
      const WP = require({json.dumps(str(cjs))});
      const model = {json.dumps(model)}; const cases = {json.dumps(cases)};
      console.log(JSON.stringify(cases.map(c => WP.predict(model, ...c))));"""
    js = json.loads(subprocess.run(["node", "-"], input=js_src, capture_output=True, text=True, check=True).stdout)
    worst = max(abs(a - b) for a, b in zip(py, js))
    assert worst < 1e-9, worst
    print(f"5000 states, max difference {worst:.2e}")

if __name__ == "__main__":
    test_parity()
