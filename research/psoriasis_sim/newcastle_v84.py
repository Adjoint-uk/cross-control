"""Faithful Python port of the Newcastle psoriasis ODE model (psor_v8_4.xml).

Shmarov, Smith, Weatherhead, Reynolds & Zuliani (2022) "Individualised computational
modelling of immune mediated disease onset, flare and clearance in psoriasis",
PLoS Comput Biol 18(9): e1010267.  Original SBML/MATLAB: github.com/pzuliani/psoriasis

Units: cells per mm^2 of epidermis, time in days, cytokines arbitrary.
"""
import numpy as np
from scipy.integrate import solve_ivp

P0 = dict(
    t2=55, t20=1.51, il172=0.5, il1720=36.5, tnf2=0.5, tnf20=36.5,
    il232=1, il2320=36.5, gf20=36.5, sc2gf=1, ta2gf=1,
    t_vm=6000, t_kp=3e12, n=4, dc_act=1880, dc_stim=0,
    sc2=0.0017635, ta2=0.0017562, sc2ta=0.0015415, ta2d=0.0018063,
    sc20=1.8135e-05, ta20=3.7321e-06, d20=5.05e-07, desq=0.004752946,
    a_base=0.0036, a20=36, uv_eff=0.16, uv_dose=0, arrest=0,
    sc_min=3947.21, ta_min=22224.4, t_min=1556.09, dc_min=1563.06,
)
NAMES = ["SC", "TA", "D", "T", "DC", "GF", "IL17", "TNF", "IL23", "A"]
Y_HEALTHY = np.array([3947.21, 22224.4, 50529.3, 1556.09, 1563.06, 717.03,
                      21.3163, 21.3163, 42.8237, 7.98201])
TOT_H, TOT_P = 79828.0, 266011.65  # totC healthy / psoriatic (from the SBML)


def rhs(t, y, p):
    SC, TA, D, T, DC, GF, IL17, TNF, IL23, A = y
    cyt = IL17 + TNF
    uv = p["uv_dose"] * p["uv_eff"]
    ab = p["a_base"]
    ap_sc = (1 - p["arrest"]) * uv * (SC - p["sc_min"]) + ab * SC
    ap_ta = (1 - p["arrest"]) * uv * (TA - p["ta_min"]) + ab * TA
    ap_t = uv * (T - p["t_min"]) + ab * T
    ap_dc = uv * (DC - p["dc_min"]) + ab * DC
    ap_d = ab * D
    dSC = p["sc2"] * cyt * SC - p["sc20"] * SC**2 - ap_sc
    dTA = p["ta2"] * cyt * TA - p["ta20"] * TA**2 + p["sc2ta"] * SC * cyt - ap_ta
    dD = p["ta2d"] * TA * cyt - p["d20"] * D**2 - ap_d - p["desq"] * D
    dT = p["t2"] * IL23 - p["t20"] * T - ap_t
    gfn = GF ** p["n"]
    dDC = p["t_vm"] * gfn / (p["t_kp"] + gfn) + p["dc_act"] + p["dc_stim"] - p["t20"] * DC - ap_dc
    dGF = p["sc2gf"] * SC + p["ta2gf"] * TA - p["gf20"] * GF
    dIL17 = p["il172"] * T - p["il1720"] * IL17
    dTNF = p["tnf2"] * T - p["tnf20"] * TNF
    dIL23 = p["il232"] * DC - p["il2320"] * IL23
    dA = ap_sc + ap_ta + ap_t + ap_dc + ap_d - p["a20"] * A
    return [dSC, dTA, dD, dT, dDC, dGF, dIL17, dTNF, dIL23, dA]


def total_cells(Y):
    return Y[0] + Y[1] + Y[2] + Y[3] + Y[4] + Y[9]


def pasi(Y, baseline=1.0):
    return baseline * (total_cells(Y) - TOT_H) / (TOT_P - TOT_H)


def simulate(events, t_end, y0=None, params=None, dt=0.05):
    """events: list of (t_start, t_stop, {param: value}) piecewise-constant inputs."""
    p = dict(P0, **(params or {}))
    y = Y_HEALTHY.copy() if y0 is None else np.asarray(y0, float)
    cuts = sorted({0.0, t_end, *[e[0] for e in events], *[e[1] for e in events]})
    cuts = [c for c in cuts if 0 <= c <= t_end]
    T, Y = [], []
    for a, b in zip(cuts[:-1], cuts[1:]):
        pp = dict(p)
        for s, e, kv in events:
            if s <= a < e:
                pp.update(kv)
        te = np.arange(a, b, dt)
        sol = solve_ivp(rhs, (a, b), y, args=(pp,), method="LSODA", t_eval=te,
                        rtol=1e-8, atol=1e-6, max_step=0.25)
        T.append(sol.t); Y.append(sol.y)
        y = solve_ivp(rhs, (a, b), y, args=(pp,), method="LSODA", rtol=1e-8, atol=1e-6, max_step=0.25).y[:, -1]
    return np.concatenate(T), np.concatenate(Y, axis=1)


def uvb_course(start, doses=(0.7, 0.7, 0.98, 0.98, 1.323, 1.323, 1.72, 1.72, 2.15, 2.15,
                              2.58, 2.58, 2.967, 2.967, 3.264, 3.264) + (3.427,) * 9,
               offsets=(0, 2, 4, 7, 9, 11, 14, 16, 18, 21, 23, 25, 28, 30, 32, 35, 37, 39,
                        42, 44, 46, 49, 51, 53, 56)):
    """The 3x/week escalating TL-01 course hard-coded in the SBML events."""
    return [(start + o, start + o + 1, {"uv_dose": d}) for o, d in zip(offsets, doses)]


if __name__ == "__main__":
    ev = [(150, 154, {"dc_stim": 3000})] + uvb_course(300)
    t, Y = simulate(ev, 450)
    for tt in [0, 149, 200, 299, 330, 360, 449]:
        i = np.searchsorted(t, tt)
        print(f"day {tt:4d}: totC={total_cells(Y[:, i]):9.0f}  PASI(rel)={pasi(Y[:, i]):.3f}")
