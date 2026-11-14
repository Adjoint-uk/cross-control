"""Extended psoriasis model built on the Newcastle v8.4 ODE core.

Additions (each reduces to the original when switched off):
  1. Pathogenic tissue-resident memory T cells (TRM) - the "molecular scar".
     Formed from excess lesional T cells, maintained by IL-23, depleted by UVB,
     and producing IL-17 locally. With TRM = 0 the model is exactly v8.4.
  2. Mechanistic biologic PK/PD: SC depot -> central compartment, Emax
     neutralisation of TNF (adalimumab), IL-17A (secukinumab), IL-23p19
     (risankizumab) or p40 (ustekinumab), on real label dosing schedules.
  3. IL-17 vs TNF potency split (w17 + wTNF = 2) which leaves the calibrated
     baseline untouched (IL17 == TNF in the original) but lets drugs differ.
  4. Virtual-patient variability and Poisson environmental triggers
     (infection / trauma / stress -> DC activation pulses) for flares/relapse.

Values marked ASSUMPTION are illustrative, not fitted to patient data.
"""
import numpy as np
from scipy.integrate import solve_ivp
from newcastle_v84 import P0 as P_CORE, Y_HEALTHY as Y_CORE, uvb_course

LN2 = np.log(2)
P_EXT = dict(P_CORE,
    # IL-17 is the dominant keratinocyte activator (ASSUMPTION: 65/35 split)
    w17=1.3, wtnf=0.7,
    # TRM module (ASSUMPTIONS, order-of-magnitude from TRM literature)
    k_form=4e-4,          # /day, TRM formation per excess lesional T cell
    d_rm0=LN2 / 1095,     # IL-23-independent loss: ~3 yr half-life
    d_rm1=LN2 / 30,       # extra loss when IL-23 survival signal is absent
    K23=5.0,              # IL-23 level giving half-maximal TRM survival
    k_rm17=0.05,          # IL-17 output per TRM relative to scale of T cells
    trm_uv=0.1,           # UVB kills TRM like other epidermal T cells
    soft=200.0,           # smoothing width of max(T - T_H, 0)
    imax=0.95,
    beta=0.0,             # cytokine-independent share of healthy keratinocyte drive
    cyt_h=2 * 21.3163,    # healthy IL17+TNF, so baseline is unchanged for any beta
)

# label regimens; PK = typical population values (approximate).
# ec50 = effective *tissue-level* serum concentration for half-maximal pathway block,
# set by calibrate.py to published wk-16 PASI90 rates. Not a binding constant.
DRUGS = {
    "adalimumab":  dict(target="TNF",  F=0.64, ka=0.28, V=7.5,  t12=14, ec50=4.78,
                        doses=[(0, 80)] + [(7 + 14 * k, 40) for k in range(80)]),
    "secukinumab": dict(target="IL17", F=0.73, ka=0.18, V=7.1,  t12=27, ec50=116.0,
                        doses=[(7 * w, 300) for w in range(5)] + [(28 + 28 * k, 300) for k in range(1, 40)]),
    "ustekinumab": dict(target="IL23", F=0.57, ka=0.20, V=15.7, t12=21, ec50=5.16,
                        doses=[(0, 45), (28, 45)] + [(28 + 84 * k, 45) for k in range(1, 15)]),
    "risankizumab": dict(target="IL23", F=0.89, ka=0.24, V=11.2, t12=28, ec50=24.1,
                        doses=[(0, 150), (28, 150)] + [(28 + 84 * k, 150) for k in range(1, 15)]),
}
NAMES = ["SC", "TA", "D", "T", "DC", "GF", "IL17", "TNF", "IL23", "A", "TRM", "depot", "central"]


def rhs(t, y, p, drug):
    SC, TA, D, T, DC, GF, IL17, TNF, IL23, A, TRM, dep, cen = y
    blk = {"TNF": 0.0, "IL17": 0.0, "IL23": 0.0}
    ddep = dcen = 0.0
    if drug is not None:
        kel = LN2 / drug["t12"]
        ddep = -drug["ka"] * dep
        dcen = drug["ka"] * dep - kel * cen
        conc = max(cen, 0) / drug["V"]                      # ug/mL
        blk[drug["target"]] = p["imax"] * conc / (conc + drug["ec50"])
    il17f, tnff, il23f = IL17 * (1 - blk["IL17"]), TNF * (1 - blk["TNF"]), IL23 * (1 - blk["IL23"])
    cyt = p["beta"] * p["cyt_h"] + (1 - p["beta"]) * (p["w17"] * il17f + p["wtnf"] * tnff)
    uv = p["uv_dose"] * p["uv_eff"]
    ab = p["a_base"]
    ap_sc = uv * (SC - p["sc_min"]) + ab * SC
    ap_ta = uv * (TA - p["ta_min"]) + ab * TA
    ap_t = uv * (T - p["t_min"]) + ab * T
    ap_dc = uv * (DC - p["dc_min"]) + ab * DC
    dSC = p["sc2"] * cyt * SC - p["sc20"] * SC**2 - ap_sc
    dTA = p["ta2"] * cyt * TA - p["ta20"] * TA**2 + p["sc2ta"] * SC * cyt - ap_ta
    dD = p["ta2d"] * TA * cyt - p["d20"] * D**2 - ab * D - p["desq"] * D
    dT = p["t2"] * il23f - p["t20"] * T - ap_t
    gfn = max(GF, 0) ** p["n"]
    dDC = p["t_vm"] * gfn / (p["t_kp"] + gfn) + p["dc_act"] + p["dc_stim"] - p["t20"] * DC - ap_dc
    dGF = p["sc2gf"] * SC + p["ta2gf"] * TA - p["gf20"] * GF
    dIL17 = p["il172"] * (T + p["k_rm17"] * TRM) - p["il1720"] * IL17
    dTNF = p["tnf2"] * T - p["tnf20"] * TNF
    dIL23 = p["il232"] * DC - p["il2320"] * IL23
    x = (T - p["t_min"]) / p["soft"]
    excess = p["soft"] * np.logaddexp(0, x) - p["soft"] * np.log(2)  # smooth max(T-T_H,0), =0 at baseline
    surv = il23f / (il23f + p["K23"])
    dTRM = p["k_form"] * max(excess, 0) - (p["d_rm0"] + p["d_rm1"] * (1 - surv)) * TRM \
        - p["trm_uv"] * uv * TRM
    dA = ap_sc + ap_ta + ap_t + ap_dc + ab * D - p["a20"] * A
    return [dSC, dTA, dD, dT, dDC, dGF, dIL17, dTNF, dIL23, dA, dTRM, ddep, dcen]


Y0 = np.concatenate([Y_CORE, [0.0, 0.0, 0.0]])


def total_cells(Y):
    return Y[0] + Y[1] + Y[2] + Y[3] + Y[4] + Y[9]


def simulate(t_end, y0=None, params=None, stim=(), drug=None, drug_start=0.0,
             drug_stop=np.inf, dt=1.0):
    """stim: list of (t_start, t_stop, {param: value}); drug: name or dict."""
    p = dict(P_EXT, **(params or {}))
    if isinstance(drug, str):
        drug = DRUGS[drug]
    y = (Y0 if y0 is None else np.asarray(y0, float)).copy()
    doses = []
    if drug is not None:
        doses = [(drug_start + td, mg * drug["F"]) for td, mg in drug["doses"]
                 if drug_start + td < min(drug_stop, t_end)]
    cuts = sorted({0.0, t_end, *[s for s, _, _ in stim], *[e for _, e, _ in stim], *[d for d, _ in doses]})
    cuts = [c for c in cuts if 0 <= c <= t_end]
    Ts, Ys = [], []
    for a, b in zip(cuts[:-1], cuts[1:]):
        for td, amt in doses:
            if td == a:
                y[11] += amt
        pp = dict(p)
        for s, e, kv in stim:
            if s <= a < e:
                pp.update(kv)
        te = np.arange(np.ceil(a / dt) * dt, b, dt)
        sol = solve_ivp(rhs, (a, b), y, args=(pp, drug), method="LSODA",
                        t_eval=np.append(te, b) if len(te) == 0 or te[-1] < b else te,
                        dense_output=False, rtol=1e-7, atol=1e-6, max_step=1.0)
        keep = sol.t < b
        Ts.append(sol.t[keep]); Ys.append(sol.y[:, keep])
        y = sol.y[:, -1].copy()
    Ts.append([t_end]); Ys.append(y[:, None])
    return np.concatenate(Ts), np.concatenate(Ys, axis=1)


def onset(params=None, settle=400):
    """Return the psoriatic state reached after the canonical 4-day DC stimulus."""
    t, Y = simulate(settle, params=params, stim=[(50, 54, {"dc_stim": 3000})], dt=5)
    return Y[:, -1]
