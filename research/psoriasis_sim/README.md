# Psoriasis systems-biology model: Newcastle v8.4 port + extensions

    pip install numpy scipy matplotlib
    python newcastle_v84.py     # faithful port, prints onset/UVB clearance
    python calibrate.py         # sets drug EC50s from published wk-16 PASI90 rates
    python experiments.py       # all figures + out/results.json (~40 s on 4 cores)

## Files
- `newcastle_v84.py`: line-for-line port of `psor_v8_4.xml` (Shmarov, Smith, Weatherhead, Reynolds,
  Zuliani, *PLoS Comput Biol* 2022, github.com/pzuliani/psoriasis). It reproduces the SBML's psoriatic
  plateau (266,012 vs 266,011.65 cells/mm²) and UVB clearance.
- `extended.py`: the extended model. With `k_form=0` it gives the same output as v8.4 (checked).
  1. Pathogenic tissue-resident memory T cells (TRM, the "molecular scar"): they form from excess
     lesional T cells, need IL-23 to survive, are partly killed by UVB and make IL-17.
  2. PK/PD for biologics: SC depot → central compartment, Emax block of TNF / IL-17A / IL-23,
     using the label dosing schedules for adalimumab, secukinumab, ustekinumab and risankizumab.
  3. IL-17 vs TNF potency split. This does not change the calibrated baseline, because IL17 ≡ TNF in v8.4.
  4. Virtual patients (screened for bistability) and Poisson environmental triggers after withdrawal.
- `out/`: four figures and `results.json`.

## Results (from out/results.json)
- **Flare threshold vs TRM:** clear skin holding 600 TRM/mm² needs 40% less trigger to flare
  (1676 → 1005). At ≥1000 TRM/mm² disease sustains itself with no trigger.
- **Fragility:** only 136/360 candidates with ±3–5% variation in two parameters stayed bistable.
  The nominal parameter set sits within ~3% of the switch-on point.
- **Virtual trial (N=120/arm):** wk-16 PASI90 is ADA 58%, SEC 77%, UST 54%, RIS 84%. These rates
  were calibrated against trial data.
- **Predictions (not fitted):**
  - IL-23 blockade leaves the least TRM at withdrawal (median 158 vs ~235–280).
  - 2-year drug-free remission came out about equal across drugs (~50%). Under these assumptions,
    clearing TRM alone does not reproduce the longer remissions seen clinically with IL-23 blockers.
    Either TRM depend much more strongly on IL-23 than assumed here, or other mechanisms
    (e.g. Treg recovery) carry that effect. This is a testable hypothesis.
- **Known mismatch:** in the model the IL-23 drug works faster than the IL-17 drug at week 4.
  Clinically IL-17 blockers are fastest. This is because v8.4 places IL-23 → T-cell expansion upstream
  with a large gain.

## Limitations
- Keratinocyte proliferation in v8.4 depends entirely on cytokines, so strong blockade pushes cell
  counts below the healthy level (PASI is clipped at 0). Adding a basal drive (`beta>0`) removes
  bistability unless the core is recalibrated.
- The TRM and trigger parameters are order-of-magnitude assumptions.
- The model has one compartment and no space, so it cannot produce plaque shape, spread or Koebnerisation.
