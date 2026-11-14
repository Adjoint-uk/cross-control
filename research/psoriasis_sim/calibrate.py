"""Set each drug's nominal EC50 so the virtual population hits published week-16 PASI90 rates.
Targets (head-to-head/phase-3 ballpark): adalimumab ~0.48 (VOYAGE-1), secukinumab ~0.70 (CLEAR),
ustekinumab ~0.45 (UltIMMa-1/2), risankizumab ~0.75 (UltIMMa-1/2).  Everything else
(onset speed, TRM, relapse after withdrawal) is then a model prediction, not a fit."""
import numpy as np
from statistics import NormalDist
import extended as E
TARGET = {"adalimumab": .48, "secukinumab": .70, "ustekinumab": .45, "risankizumab": .75}
SIGMA = 0.8  # lognormal SD of patient EC50 multiplier (must match experiments.py)


def clears(drug, ec50, t0=3 * 365):
    d = dict(E.DRUGS[drug], ec50=ec50)
    t, Y = E.simulate(t0 + 113, stim=[(10, 14, {"dc_stim": 3000})], drug=d, drug_start=t0, dt=7)
    tot = E.total_cells(Y); P = tot[np.searchsorted(t, t0) - 1]
    return (tot[-1] - 79828) / (P - 79828) <= 0.10


def critical(drug):
    lo, hi = 1e-3, 1e3
    for _ in range(30):
        m = np.sqrt(lo * hi)
        lo, hi = (m, hi) if clears(drug, m) else (lo, m)
    return lo


if __name__ == "__main__":
    for d, tg in TARGET.items():
        c = critical(d)
        nom = c * np.exp(-SIGMA * NormalDist().inv_cdf(tg))
        print(f'"{d}": critical EC50 {c:.3g} ug/mL -> nominal {nom:.3g}')
