"""Run all experiments and write figures + results.json into ./out"""
import json, os
import numpy as np
from multiprocessing import Pool
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import newcastle_v84 as nc
import extended as E

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
os.makedirs(OUT, exist_ok=True)
COL = {"adalimumab": "#c0504d", "secukinumab": "#4f81bd", "ustekinumab": "#9bbb59",
       "risankizumab": "#8064a2"}
RES = {}


# ---------- 1. reproduction of the Newcastle model ----------
def fig_reproduction():
    ev = [(150, 154, {"dc_stim": 3000})] + nc.uvb_course(300)
    t, Y = nc.simulate(ev, 450, dt=0.25)
    t2, Y2 = E.simulate(900, stim=[(150, 154, {"dc_stim": 3000})] + nc.uvb_course(300), dt=1)
    fig, ax = plt.subplots(1, 2, figsize=(11, 4))
    ax[0].plot(t / 7, nc.total_cells(Y), "k")
    ax[0].axvspan(150 / 7, 154 / 7, color="orange", alpha=.4, label="4-day DC stimulus")
    ax[0].axvspan(300 / 7, 357 / 7, color="violet", alpha=.25, label="UVB course")
    ax[0].set(xlabel="weeks", ylabel="keratinocytes + immune cells / mm²",
              title="Newcastle v8.4 (Python port): onset & UVB clearance")
    ax[0].legend(fontsize=8)
    ax[1].plot(t2 / 7, E.total_cells(Y2), "k", label="total cells")
    a2 = ax[1].twinx(); a2.plot(t2 / 7, Y2[10], color="purple", label="TRM"); a2.set_ylabel("TRM / mm²", color="purple")
    ax[1].set(xlabel="weeks", title="Extended model: TRM 'scar' outlives the plaque")
    fig.tight_layout(); fig.savefig(f"{OUT}/1_reproduction.png", dpi=130); plt.close(fig)
    RES["reproduction"] = {"psoriatic_plateau_totC": float(nc.total_cells(Y)[np.searchsorted(t, 299)]),
                           "sbml_totC_max": nc.TOT_P,
                           "post_uvb_totC_day449": float(nc.total_cells(Y)[np.searchsorted(t, 449)])}


# ---------- 2. hysteresis / bistability vs DC activation, at fixed TRM ----------
def _branch(trm):
    p = dict(k_form=0, d_rm0=0, d_rm1=0)
    grid = np.linspace(800, 3200, 41)
    out = {}
    for direction, g in (("up", grid), ("down", grid[::-1])):
        y = E.Y0.copy(); y[10] = trm
        if direction == "down":
            y = E.simulate(400, y0=y, params=dict(p, dc_act=g[0]), dt=50)[1][:, -1]
        vals = []
        for v in g:
            y = E.simulate(400, y0=y, params=dict(p, dc_act=v), dt=100)[1][:, -1]
            vals.append(E.total_cells(y))
        out[direction] = (g.tolist(), vals)
    return trm, out


def fig_hysteresis(pool):
    res = pool.map(_branch, [0, 500, 1000])
    fig, ax = plt.subplots(figsize=(6.5, 4.3))
    for (trm, o), c in zip(res, ["k", "tab:orange", "tab:red"]):
        ax.plot(*o["up"], "-", color=c, label=f"TRM={trm}: increasing")
        ax.plot(*o["down"], "--", color=c, label=f"TRM={trm}: decreasing")
    ax.axvline(1880, color="grey", lw=.8, ls=":"); ax.text(1890, 1.1e5, "nominal", fontsize=8, color="grey")
    ax.set(xlabel="baseline DC activation (dc_act)", ylabel="steady-state total cells / mm²",
           title="Bistability & hysteresis: TRM shift the switch left")
    ax.legend(fontsize=7); fig.tight_layout(); fig.savefig(f"{OUT}/2_hysteresis.png", dpi=130); plt.close(fig)
    RES["hysteresis"] = {str(trm): {"switch_on_dc_act": float(next(g for g, v in zip(*o["up"]) if v > 1.5e5)),
                                    "switch_off_dc_act": float(next(g for g, v in zip(*o["down"]) if v < 1.5e5))}
                         for trm, o in res}


# ---------- 3. flare threshold vs TRM ----------
def _threshold(trm):
    def flares(s):
        y = E.Y0.copy(); y[10] = trm
        Y = E.simulate(300, y0=y, params=dict(k_form=0, d_rm0=0, d_rm1=0),
                       stim=[(10, 14, {"dc_stim": s})] if s > 0 else [], dt=50)[1]
        return E.total_cells(Y[:, -1]) > 1.5e5
    if flares(0):
        return 0.0
    lo, hi = 0.0, 5000.0
    for _ in range(12):
        m = (lo + hi) / 2
        lo, hi = (lo, m) if flares(m) else (m, hi)
    return hi


def fig_threshold(pool):
    trms = np.linspace(0, 1600, 17)
    thr = pool.map(_threshold, trms)
    fig, ax = plt.subplots(figsize=(6, 4))
    ax.plot(trms, thr, "o-k")
    ax.set(xlabel="pathogenic TRM in clinically clear skin (cells/mm²)",
           ylabel="4-day trigger needed to re-flare (dc_stim)",
           title="Memory T cells lower the relapse threshold")
    fig.tight_layout(); fig.savefig(f"{OUT}/3_flare_threshold.png", dpi=130); plt.close(fig)
    RES["flare_threshold"] = dict(zip([float(x) for x in trms], [float(x) for x in thr]))


# ---------- 4+5. virtual trial: 52 weeks on drug, then withdrawal with random triggers ----------
N = 120
DUR_ON, FOLLOW = 364, 2 * 364


def make_patients(seed=1):
    r = np.random.default_rng(seed)
    pts = []
    for i in range(3 * N):
        pts.append(dict(
            params=dict(t_vm=6000 * r.lognormal(0, .05), t2=55 * r.lognormal(0, .03),
                        k_form=4e-4 * r.lognormal(0, .4), k_rm17=0.05 * r.lognormal(0, .3)),
            pk=dict(ec50_mult=r.lognormal(0, .8), cl_mult=r.lognormal(0, .3)),
            disease_years=r.uniform(1, 6),
            triggers=[(float(t), float(600 * r.lognormal(0, .6)))
                      for t in np.cumsum(r.exponential(90, 40)) if t < FOLLOW],
        ))
    return pts


def _screen(pt):
    """Keep only patients that are bistable: healthy at rest, psoriatic after the trigger."""
    p = dict(pt["params"])
    yH = E.simulate(600, params=p, dt=100)[1][:, -1]
    p["t_min"] = yH[3]
    yP = E.simulate(500, y0=yH, params=p, stim=[(10, 14, {"dc_stim": 3000})], dt=100)[1][:, -1]
    return E.total_cells(yH) < 1e5 and E.total_cells(yP) > 2e5


def _run(args):
    pt, drug = args
    p = dict(pt["params"])
    yH = E.simulate(600, params=p, dt=100)[1][:, -1]          # patient's own healthy state
    p.update(t_min=yH[3], sc_min=yH[0], ta_min=yH[1], dc_min=yH[4])
    H = E.total_cells(yH)
    t0 = 20 + pt["disease_years"] * 365
    d = None
    if drug:
        d = dict(E.DRUGS[drug]); d["ec50"] *= pt["pk"]["ec50_mult"]; d["t12"] /= pt["pk"]["cl_mult"]
    stop = t0 + DUR_ON
    stim = [(10, 14, {"dc_stim": 3000})] + [(stop + a, stop + a + 4, {"dc_stim": m}) for a, m in pt["triggers"]]
    t, Y = E.simulate(stop + FOLLOW, y0=yH, params=p, stim=stim, drug=d, drug_start=t0, drug_stop=stop, dt=7)
    tot = E.total_cells(Y)
    P = tot[np.searchsorted(t, t0) - 1]
    rel = np.clip((tot - H) / (P - H), 0, None)                # relative PASI (1 = baseline)
    tt = (t - t0) / 7
    def at(w): return float(rel[np.searchsorted(tt, w)])
    # time to relapse (loss of PASI50) after stopping, only for those clear (<PASI75) at stop
    after = (tt > DUR_ON / 7)
    clear_at_stop = at(DUR_ON / 7 - 0.5) <= 0.25
    rel_idx = np.where(after & (rel >= 0.5))[0]
    relapse_wk = float(tt[rel_idx[0]] - DUR_ON / 7) if (clear_at_stop and len(rel_idx)) else None
    G = np.arange(-8, (DUR_ON + FOLLOW) / 7, 1.0)
    return dict(drug=drug, wk=G.tolist(), rel=np.interp(G, tt, rel).tolist(), w4=at(4), w12=at(12), w16=at(16), w52=at(51.5),
                trm_base=float(Y[10, np.searchsorted(t, t0) - 1]), trm_stop=float(Y[10, np.searchsorted(t, stop) - 1]),
                clear_at_stop=bool(clear_at_stop), relapse_wk=relapse_wk)


def km(times, censor_at):
    """Kaplan-Meier with administrative censoring (None => no event)."""
    ev = sorted((t if t is not None else censor_at, t is not None) for t in times)
    s, at_risk, xs, ys = 1.0, len(ev), [0], [1.0]
    for t, e in ev:
        if e:
            s *= (at_risk - 1) / at_risk; xs.append(t); ys.append(s)
        at_risk -= 1
    xs.append(censor_at); ys.append(s)
    return xs, ys


def fig_trial(pool):
    cand = make_patients()
    ok = pool.map(_screen, cand)
    pts = [c for c, k in zip(cand, ok) if k][:N]
    RES["virtual_population"] = {"candidates": len(cand), "bistable": int(sum(ok)), "used": len(pts)}
    drugs = list(E.DRUGS)
    res = pool.map(_run, [(p, d) for d in drugs for p in pts])
    by = {d: [r for r in res if r["drug"] == d] for d in drugs}
    summ = {}
    fig, ax = plt.subplots(1, 3, figsize=(16, 4.5))
    for d in drugs:
        R = by[d]
        W = np.array(R[0]["wk"]); M = np.array([r["rel"] for r in R])
        m = (W > -8) & (W < (DUR_ON + FOLLOW) / 7)
        ax[0].plot(W[m], np.median(M, 0)[m] * 100, color=COL[d], label=d)
        ax[0].fill_between(W[m], np.percentile(M, 25, 0)[m] * 100, np.percentile(M, 75, 0)[m] * 100, color=COL[d], alpha=.12)
        clear = [r for r in R if r["clear_at_stop"]]
        xs, ys = km([r["relapse_wk"] for r in clear], FOLLOW / 7)
        ax[2].step(xs, ys, where="post", color=COL[d], label=f"{d} (n={len(clear)})")
        ax[1].scatter([d] * len(R), [r["trm_stop"] for r in R], s=6, color=COL[d], alpha=.5)
        rw = [r["relapse_wk"] for r in clear]
        summ[d] = dict(
            PASI75_wk4=float(np.mean([r["w4"] <= .25 for r in R])), PASI75_wk16=float(np.mean([r["w16"] <= .25 for r in R])),
            PASI90_wk16=float(np.mean([r["w16"] <= .10 for r in R])), PASI90_wk52=float(np.mean([r["w52"] <= .10 for r in R])),
            median_TRM_before=float(np.median([r["trm_base"] for r in R])), median_TRM_at_stop=float(np.median([r["trm_stop"] for r in R])),
            n_clear_at_stop=len(clear), relapse_free_2y=float(ys[-1]),
            median_wk_to_relapse_among_relapsers=float(np.median([x for x in rw if x is not None])) if any(x is not None for x in rw) else None)
    ax[0].axvline(DUR_ON / 7, color="grey", ls=":"); ax[0].text(DUR_ON / 7 + 1, 90, "drug stopped", fontsize=8)
    ax[0].set(xlabel="weeks from first dose", ylabel="PASI, % of baseline (median, IQR)", title=f"Virtual trial, N={N}/arm", ylim=(0, 110))
    ax[0].legend(fontsize=8)
    ax[1].set(ylabel="TRM / mm² at withdrawal", title="Residual TRM 'scar' at week 52"); ax[1].tick_params(axis="x", rotation=20)
    ax[2].set(xlabel="weeks after withdrawal", ylabel="relapse-free (PASI50 kept)", title="Drug-free remission (Kaplan–Meier)", ylim=(0, 1.02))
    ax[2].legend(fontsize=8)
    fig.tight_layout(); fig.savefig(f"{OUT}/4_virtual_trial.png", dpi=130); plt.close(fig)
    RES["virtual_trial"] = summ


if __name__ == "__main__":
    fig_reproduction()
    with Pool() as pool:
        fig_threshold(pool)
        fig_hysteresis(pool)
        fig_trial(pool)
    json.dump(RES, open(f"{OUT}/results.json", "w"), indent=1)
    print(json.dumps({k: v for k, v in RES.items() if k != "flare_threshold"}, indent=1))
    print("thresholds", {k: round(v) for k, v in RES["flare_threshold"].items()})
