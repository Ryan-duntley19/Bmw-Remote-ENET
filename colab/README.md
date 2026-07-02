# All-in-One Colab Fine-Tune

**`qwen36_27b_finetune_colab.ipynb`** is the single-file deliverable: upload
it to Google Colab, optionally adjust the CONFIGURATION cell, and
`Runtime → Run all`. It performs the entire pipeline — environment checks,
dependency install, dataset download and Hermes-format conversion, the
mask-verification gate, LoRA SFT with Unsloth (checkpointed, auto-resuming),
merge, GGUF export, and optional Hugging Face upload.

**Primary target: the Colab Pro G4 instance** (RTX PRO 6000, 96GB VRAM /
180GB RAM / ~200GB disk) — the defaults (27B bf16 LoRA at 16K, checkpoints
on local VM disk, no Google Drive) run as-is there. An auto-adjust cell
detects weaker runtimes and downshifts safely: A100-40GB-class → QLoRA +
seq 8192; free-tier T4 → hard-stops for models over ~9B with a clear
message, otherwise 4-bit + fp16 + seq 4096 + frequent checkpoints. Google
Drive is opt-in (`USE_DRIVE = True`) and only ever receives checkpoints and
the adapter, with retention auto-sized to the account's actual free space —
a 15GB Drive is fine; the merged model and GGUFs never touch Drive.

## Do not edit the notebook directly

It is **generated**. The embedded pipeline code is read verbatim from
`training/trainkit/` at build time so it always matches the unit-tested
sources. To change anything:

```bash
# edit colab/build_notebook.py (notebook cells) or training/trainkit/ (pipeline)
python colab/build_notebook.py     # regenerate
python colab/validate_notebook.py  # verify (JSON, syntax, module identity, config drift)
```

`validate_notebook.py` fails the build if an embedded module ever drifts
from its tested source, if any cell has a syntax error, or if a
configuration knob is used but no longer defined.

## What the notebook assumes

- `MODEL_ID = "Qwen/Qwen3.6-27B"` — verify the exact repo id on hf.co
  before the run. On sub-20GB GPUs a ~4–8B `MODEL_ID` (+ matching
  `MODEL_SIZE_B`) is required; the notebook enforces this.
- Data is downloaded from public, non-gated Hugging Face datasets and
  converted defensively (per-source yield reported, licenses recorded in
  `data/manifest.json`). A `personal.jsonl` uploaded to `/content/data/`
  is mixed in automatically as the specialization slice.

## Relationship to the rest of the repo

The notebook is the "just works" packaging of the same pipeline:
`training/` holds the script-based version (plus the ORPO stage and the
QLoRA config), `evals/` holds the Phase 6 evaluation suite for judging the
result against the stock model.
