#!/usr/bin/env python3
"""Generate the all-in-one Colab fine-tuning notebook.

The notebook must run on a fresh Colab VM with nothing but itself, so the
unit-tested trainkit modules (chat_format, masking, packing, data,
mask_check) are embedded verbatim as %%writefile cells — read from
training/trainkit/ at build time, never hand-copied. Rebuild after any
trainkit change:

    python colab/build_notebook.py

Output: colab/qwen36_27b_finetune_colab.ipynb
"""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRAINKIT = ROOT / "training" / "trainkit"
OUT = Path(__file__).resolve().parent / "qwen36_27b_finetune_colab.ipynb"

EMBEDDED_MODULES = ["chat_format.py", "masking.py", "packing.py", "data.py",
                    "mask_check.py"]


def md(source: str) -> dict:
    return {"cell_type": "markdown", "metadata": {}, "source": source}


def py(source: str) -> dict:
    return {"cell_type": "code", "metadata": {}, "execution_count": None,
            "outputs": [], "source": source}


cells = []

# ===========================================================================
cells.append(md(
"""# Qwen3.6-27B → Local Agentic Engineering Assistant — All-in-One Colab Fine-Tune

This single notebook runs the **entire pipeline** on a Colab instance with a 96GB-VRAM GPU (180GB RAM / 200GB disk):

1. Environment checks and dependency install
2. **Data preparation** — downloads a Plan-A-shaped mixture (tool-calling / code / math / general) from public Hugging Face datasets and converts it to the Hermes chat format
3. **Mask-verification gate** — hard pre-flight proof that loss lands only on assistant tokens (runs before the model is ever downloaded)
4. **bf16 LoRA SFT with Unsloth** (r=64, response-only loss, FFD sequence packing, checkpoint/auto-resume)
5. Quick generation smoke test
6. **Merge** LoRA → bf16 checkpoint (CPU-side), reclaim disk
7. **GGUF export** (Q4_K_M by default) via llama.cpp
8. Optional upload to the Hugging Face Hub

**How to use:** set your knobs in the CONFIGURATION cell, then `Runtime → Run all`. Expected runtime at the default data mix: roughly 1 hour of setup/data + ~10–20 GPU-hours of training + ~1–2 hours merge/export.

**If the session dies mid-training:** just `Run all` again. With Google Drive enabled (default), checkpoints persist across sessions and training auto-resumes from the last complete checkpoint. Data prep re-downloads (the VM disk is ephemeral) but the completed steps are cheap relative to training.

## Hardware tiers — read this first

The notebook auto-detects your GPU and adjusts (precision, 4-bit, sequence length, checkpoint cadence). What each tier can actually do:

**Primary target: the Colab Pro G4 instance (RTX PRO 6000, 96GB VRAM / 180GB RAM / ~200GB disk).** The defaults are tuned for exactly that box and run as-is: 27B bf16 LoRA at 16K sequences, checkpoints on local VM disk, no Google Drive involved.

The auto-adjust cell is a safety net for weaker runtimes only — on the G4 it detects the big-GPU tier and changes nothing. For reference:

| Runtime | VRAM | Feasible | Settings the notebook applies |
|---|---|---|---|
| **G4 / 96GB class** | 90GB+ | 27B bf16 LoRA (best quality) | **defaults as-is** |
| A100-40GB class | 24–48GB | 27B QLoRA | forces `LOAD_IN_4BIT`, seq 8192 |
| Free tier (T4) | 16GB | ~4–8B model only, QLoRA, fp16 | forces 4-bit, seq 4096; hard-stops if `MODEL_SIZE_B` > 9 |

**Checkpoint persistence:** with `USE_DRIVE = False` (default), checkpoints live on the VM's local disk — auto-resume works within the session (rerun the training cell after a crash), but a fully recycled VM starts over. That is a deliberate trade for a ~15GB Drive account, where the big artifacts wouldn't fit anyway. If you want cross-session insurance, the end-of-run adapter (~1GB) is the thing to save: it fits Drive or uploads to the HF Hub in seconds, and everything downstream (merge, GGUF) can be re-derived from it."""))

# ===========================================================================
cells.append(md("## Step 0 — Environment check"))

cells.append(py(
"""# Detect the GPU, disk and RAM. Later cells adapt to what is found here.
import shutil, subprocess

gpu_query = subprocess.run(
    ["nvidia-smi", "--query-gpu=name,memory.total,compute_cap",
     "--format=csv,noheader,nounits"],
    capture_output=True, text=True,
)
assert gpu_query.returncode == 0, "No GPU found — set Runtime > Change runtime type to a GPU instance."
gpu_name, vram_mib, compute_cap = [s.strip() for s in gpu_query.stdout.strip().rsplit(",", 2)]
GPU_VRAM_GB = int(vram_mib) / 1024
# bf16 needs compute capability >= 8.0 (Ampere+). A T4 is 7.5 -> fp16 fallback.
BF16_OK = float(compute_cap) >= 8.0
print(f"GPU : {gpu_name} ({GPU_VRAM_GB:.0f} GB VRAM, "
      f"compute {compute_cap}, {'bf16' if BF16_OK else 'fp16 only'})")

disk = shutil.disk_usage("/")
DISK_FREE_GB = disk.free / 1024**3
print(f"Disk: {DISK_FREE_GB:.0f} GB free of {disk.total / 1024**3:.0f} GB")

with open("/proc/meminfo") as fh:
    RAM_GB = int(fh.readline().split()[1]) / 1024**2
print(f"RAM : {RAM_GB:.0f} GB")

if GPU_VRAM_GB < 20:
    print("\\nTier: FREE (T4-class). A 27B model does NOT fit this GPU in any")
    print("mode. Use a ~4-8B model: set MODEL_ID and MODEL_SIZE_B in the next")
    print("cell. The auto-adjust cell will enforce this and pick safe settings.")
elif GPU_VRAM_GB < 60:
    print("\\nTier: MID (A100-40GB class). 27B is feasible as QLoRA only;")
    print("auto-adjust will set 4-bit + seq 8192.")
else:
    print("\\nTier: BIG (90GB+ class). 27B bf16 LoRA runs with the defaults.")"""))

# ===========================================================================
cells.append(md("""## Step 1 — Configuration

Everything you might want to change is in this one cell. The training numbers are the Plan A values (justified in the project's `docs/hyperparameters.md`): LoRA r=64/α=128 on all linear projections, LR 1e-4 cosine with a 10% floor, ~131K tokens per optimizer step, 2 epochs, 8-bit Adam, Unsloth gradient checkpointing."""))

cells.append(py(
"""# ============================== CONFIGURATION ==============================

# --- model -----------------------------------------------------------------
# FREE TIER (T4 16GB)? A 27B does not fit — point MODEL_ID at a ~4-8B variant
# of the same family (check Qwen's HF org for the exact repo id) and set
# MODEL_SIZE_B to match, e.g.:
#   MODEL_ID = "Qwen/Qwen3.6-8B"; MODEL_SIZE_B = 8
MODEL_ID = "Qwen/Qwen3.6-27B"   # verify the exact repo id on hf.co before running
MODEL_SIZE_B = 27               # billions of params — drives VRAM/disk/RAM guards
MAX_SEQ_LEN = 16384
LOAD_IN_4BIT = False            # True = QLoRA (auto-adjust forces this on <60GB GPUs)
AUTO_ADJUST = True              # downshift settings to fit the detected GPU

# --- LoRA ------------------------------------------------------------------
LORA_R = 64
LORA_ALPHA = 128
TARGET_MODULES = ["q_proj", "k_proj", "v_proj", "o_proj",
                  "gate_proj", "up_proj", "down_proj"]

# --- optimization ----------------------------------------------------------
LEARNING_RATE = 1e-4
NUM_EPOCHS = 2.0
BATCH_SIZE = 1                  # one packed 16K row per micro-step
GRAD_ACCUM = 8                  # -> ~131K tokens per optimizer step
WARMUP_RATIO = 0.03
WEIGHT_DECAY = 0.01
MIN_LR_RATIO = 0.1              # cosine decays to 10% of peak, not to zero
SEED = 3407

# --- checkpointing ---------------------------------------------------------
SAVE_STEPS = 100                # ~1 checkpoint per 45-60 min
SAVE_TOTAL_LIMIT = 3
LOGGING_STEPS = 5
USE_DRIVE = False               # G4 default: local disk. True = checkpoints to
                                # Drive (only useful on small models / small
                                # checkpoints — a 15GB Drive fits ~5 of the
                                # 27B's ~2.4GB checkpoints at most)

# --- data mixture (examples per source; see the data-prep cell) -------------
N_TOOL_CALLING = 8000           # Hermes-format function calling
N_CODE = 6000                   # code instruction data
N_MATH = 4000                   # math reasoning
N_GENERAL = 2000                # general chat (anti-regression)
EVAL_FRACTION = 0.005

# --- export ----------------------------------------------------------------
GGUF_QUANTS = ["Q4_K_M"]        # add "Q6_K", "Q8_0" only if disk headroom allows

# --- optional Hugging Face upload -------------------------------------------
HF_TOKEN = ""                   # or leave empty and use interactive login
HF_UPLOAD_REPO = ""             # e.g. "yourname/qwen36-27b-engineering-assistant"

print("Configuration loaded.")
print(f"  model={MODEL_ID}  seq={MAX_SEQ_LEN}  4bit={LOAD_IN_4BIT}")
print(f"  mixture: tool={N_TOOL_CALLING} code={N_CODE} math={N_MATH} general={N_GENERAL}")"""))

# ===========================================================================
cells.append(md("""## Step 1b — Auto-adjust to the detected hardware

Turns the CONFIGURATION values into something that actually fits the GPU this session got. On a free-tier T4 this is the cell that keeps you honest: it **hard-stops** if `MODEL_SIZE_B` is too big for the card (better a clear error now than a cryptic OOM after an hour of downloads), forces QLoRA + fp16-safe settings, shortens sequences, and checkpoints more often because free sessions disconnect more."""))

cells.append(py(
"""if AUTO_ADJUST:
    if GPU_VRAM_GB < 20:  # free tier / T4-class
        assert MODEL_SIZE_B <= 9, (
            f"MODEL_SIZE_B={MODEL_SIZE_B} cannot fit a {GPU_VRAM_GB:.0f}GB GPU in any "
            "mode (4-bit weights alone exceed VRAM). On the free tier, set MODEL_ID "
            "to a ~4-8B variant and MODEL_SIZE_B to match — or buy pay-as-you-go "
            "compute units (works on a free account) for a bigger runtime.")
        LOAD_IN_4BIT = True
        MAX_SEQ_LEN = min(MAX_SEQ_LEN, 4096)
        GRAD_ACCUM = max(GRAD_ACCUM, 16)   # keep ~65K tokens per optimizer step
        SAVE_STEPS = min(SAVE_STEPS, 50)   # free sessions die more often
        SAVE_TOTAL_LIMIT = 2               # 15GB free Drive tier
        print(f"free-tier profile: 4-bit base, seq {MAX_SEQ_LEN}, "
              f"grad-accum {GRAD_ACCUM}, checkpoint every {SAVE_STEPS} steps")
    elif GPU_VRAM_GB < 60:  # A100-40GB class (paid compute units)
        if MODEL_SIZE_B > 10 and not LOAD_IN_4BIT:
            LOAD_IN_4BIT = True
            print("mid-tier profile: forcing 4-bit (QLoRA) — "
                  f"{MODEL_SIZE_B}B bf16 LoRA needs ~90GB+")
        MAX_SEQ_LEN = min(MAX_SEQ_LEN, 8192)
        print(f"mid-tier profile: seq {MAX_SEQ_LEN}")
    else:
        print("big-GPU profile: configuration used as-is")

if not BF16_OK:
    print("GPU has no bf16 support — training and model load will use fp16")

# Sanity: rough VRAM need = weights + LoRA/optimizer + activations headroom.
weights_gb = MODEL_SIZE_B * (0.65 if LOAD_IN_4BIT else 2.1)
assert weights_gb + 4 < GPU_VRAM_GB, (
    f"~{weights_gb:.0f}GB of weights will not fit {GPU_VRAM_GB:.0f}GB VRAM — "
    "reduce MODEL_SIZE_B / pick a smaller MODEL_ID, or set LOAD_IN_4BIT = True.")
print(f"estimated weight footprint: ~{weights_gb:.0f}GB of {GPU_VRAM_GB:.0f}GB VRAM")"""))

# ===========================================================================
cells.append(md("""## Step 2 — Output paths

Default (`USE_DRIVE = False`, the G4 profile): everything on the VM's local disk. Auto-resume works within the session — if training crashes, rerun the cells and it continues from the last checkpoint. If the VM itself gets recycled, the run restarts, so don't let a finished adapter sit around: download it or push it to the HF Hub (Step 12) when training ends.

If you flip `USE_DRIVE = True` (worthwhile mainly for smaller models), this cell measures your actual free Drive space, sizes checkpoint retention to fit (a 15GB account holds a handful of the 27B's ~2.4GB checkpoints), and falls back to local disk with a clear warning if Drive is critically full. Big artifacts (merged model, GGUFs) never touch Drive either way."""))

cells.append(py(
"""import os, shutil

# A trainer checkpoint = adapter + optimizer moments; rough size by model scale.
CKPT_EST_GB = max(0.5, MODEL_SIZE_B * 0.09)

RUN_ROOT = "/content/qwen36_finetune"
if USE_DRIVE:
    try:
        from google.colab import drive
        drive.mount("/content/drive")
        drive_free_gb = shutil.disk_usage("/content/drive/MyDrive").free / 1024**3
        print(f"Drive free space: {drive_free_gb:.1f} GB")
        if drive_free_gb < CKPT_EST_GB * 1.5 + 1:
            print(f"*** Drive nearly full (< {CKPT_EST_GB * 1.5 + 1:.0f}GB free) — "
                  "checkpoints would fail mid-write. Using LOCAL disk instead;")
            print("*** resume only works within this session. Free Drive space "
                  "and rerun this cell to get cross-session resume back.")
        else:
            RUN_ROOT = "/content/drive/MyDrive/qwen36_finetune"
            # Fit retention to the space that is actually there (adapter ~1GB
            # also lands on Drive at the end, hence the -1).
            fit = int((drive_free_gb - 1) // CKPT_EST_GB)
            if fit < SAVE_TOTAL_LIMIT:
                SAVE_TOTAL_LIMIT = max(1, fit)
                print(f"Drive budget: keeping {SAVE_TOTAL_LIMIT} checkpoint(s) "
                      f"(~{CKPT_EST_GB:.1f}GB each)")
    except Exception as err:  # not running in Colab, or user declined
        print(f"Drive mount failed ({err}); falling back to local disk.")

OUTPUT_DIR = f"{RUN_ROOT}/runs/sft"            # trainer checkpoints
ADAPTER_DIR = f"{RUN_ROOT}/adapter_final"      # final LoRA adapter
MERGED_DIR = "/content/merged_bf16"            # big -> local disk only, never Drive
GGUF_DIR = "/content/gguf"                     # local disk only, never Drive
DATA_PATH = "/content/data/train.jsonl"

for d in (OUTPUT_DIR, os.path.dirname(DATA_PATH)):
    os.makedirs(d, exist_ok=True)
print(f"checkpoints -> {OUTPUT_DIR}  (keep {SAVE_TOTAL_LIMIT})")
print(f"adapter     -> {ADAPTER_DIR}")"""))

# ===========================================================================
cells.append(md("## Step 3 — Install dependencies\n\nUnsloth is installed first (its installer manages the torch/transformers pairing it patches). Re-running is safe."))

cells.append(py(
"""%pip install -q --upgrade unsloth
%pip install -q "transformers>=4.46" "datasets>=3.0" "trl>=1.0" "peft>=0.14" "accelerate>=1.0" "bitsandbytes>=0.44" sentencepiece protobuf huggingface_hub gguf

# Fail here, not 40 minutes into a run.
import importlib
for mod in ("unsloth", "trl", "peft", "datasets", "transformers"):
    importlib.import_module(mod)
    print(f"ok: {mod}")

import torch
print(f"torch {torch.__version__}, cuda={torch.cuda.is_available()}, "
      f"bf16={torch.cuda.is_bf16_supported() if torch.cuda.is_available() else 'n/a'}")

if HF_TOKEN:
    from huggingface_hub import login
    login(token=HF_TOKEN)"""))

# ===========================================================================
cells.append(md("""## Step 4 — Embedded pipeline code

The next cells write the project's unit-tested `trainkit` modules to disk (60 tests in the source repo). They implement the pieces that make or break agentic SFT: ChatML rendering with per-segment loss flags, **response-only masking built by construction**, FFD sequence packing with per-example position reset, and the **mask-verification gate** that refuses to train if loss would land on the wrong tokens. Don't edit these unless you know exactly why."""))

cells.append(py(
"""import os
os.makedirs("trainkit", exist_ok=True)
with open("trainkit/__init__.py", "w") as fh:
    fh.write("# embedded trainkit package (generated from the source repo)\\n")
print("trainkit/ package created")"""))

for name in EMBEDDED_MODULES:
    source = (TRAINKIT / name).read_text(encoding="utf-8")
    cells.append(py(f"%%writefile trainkit/{name}\n{source}"))

# ===========================================================================
cells.append(md("""## Step 5 — Data preparation

Downloads and converts a Plan-A-shaped mixture from **public, non-gated** Hugging Face datasets into the Hermes `messages` format:

| Slice | Source | Why |
|---|---|---|
| tool calling | `NousResearch/hermes-function-calling-v1` (fallback: `Team-ACE/ToolACE`) | exact Hermes `<tool_call>` schema discipline |
| code | `ise-uiuc/Magicoder-OSS-Instruct-75K` | retention of code ability |
| math | `nvidia/OpenMathInstruct-2` (streamed) | STEM reasoning traces |
| general | `HuggingFaceH4/ultrachat_200k` (streamed) | anti-regression general chat |

Each source is converted defensively (rows that don't map cleanly are counted and skipped, never half-converted), exact-deduplicated, trimmed to end on an assistant turn, and tagged with its source. A source that fails to download is skipped with a loud warning rather than killing the run. Licenses are recorded from the Hub at download time into `data/manifest.json`.

**Note on the specialization slice:** the project's Phase 2 plan reserves ~10% for your own data (OpenCode session logs, your repos, runbooks). If you have `personal.jsonl` in the same `messages` format, upload it to `/content/data/personal.jsonl` before running this cell and it will be mixed in automatically."""))

cells.append(py(
r"""import hashlib, json, random, traceback

from datasets import load_dataset
from huggingface_hub import HfApi

random.seed(SEED)

ROLE_MAP = {
    "system": "system", "human": "user", "user": "user",
    "gpt": "assistant", "assistant": "assistant", "model": "assistant",
    "tool": "tool", "observation": "tool", "function_response": "tool",
}


def norm_messages(turns):
    # sharegpt-style (from/value) or openai-style (role/content) -> our schema
    msgs = []
    for t in turns:
        role = t.get("from") if "from" in t else t.get("role")
        content = t.get("value") if "value" in t else t.get("content")
        if not isinstance(role, str) or not isinstance(content, str):
            return None
        mapped = ROLE_MAP.get(role.lower())
        if mapped is None:
            return None
        content = content.strip()
        if not content:
            continue
        msgs.append({"role": mapped, "content": content})
    while msgs and msgs[-1]["role"] != "assistant":
        msgs.pop()  # nothing to train on after the last assistant turn
    if len(msgs) < 2 or not any(m["role"] == "assistant" for m in msgs):
        return None
    return msgs


def from_sharegpt(row):
    turns = row.get("conversations") or row.get("messages")
    if not isinstance(turns, list):
        return None
    msgs = norm_messages(turns)
    if msgs is None:
        return None
    # some FC datasets keep the system prompt in its own column
    sys_col = row.get("system")
    if isinstance(sys_col, str) and sys_col.strip() and msgs[0]["role"] != "system":
        msgs = [{"role": "system", "content": sys_col.strip()}] + msgs
    return msgs


def from_prompt_response(prompt_key, response_key):
    def convert(row):
        p, r = row.get(prompt_key), row.get(response_key)
        if not isinstance(p, str) or not isinstance(r, str) or not p.strip() or not r.strip():
            return None
        return [{"role": "user", "content": p.strip()},
                {"role": "assistant", "content": r.strip()}]
    return convert


# Each source lists candidate (repo, config, split) tuples tried in order.
SOURCES = [
    {"name": "tool_calling", "target": N_TOOL_CALLING, "convert": from_sharegpt,
     "streaming": False,
     "candidates": [
         ("NousResearch/hermes-function-calling-v1", "func_calling", "train"),
         ("NousResearch/hermes-function-calling-v1", "glaive_func_calling", "train"),
         ("NousResearch/hermes-function-calling-v1", "func_calling_singleturn", "train"),
         ("Team-ACE/ToolACE", None, "train"),
     ]},
    {"name": "code", "target": N_CODE,
     "convert": from_prompt_response("problem", "solution"), "streaming": False,
     "candidates": [("ise-uiuc/Magicoder-OSS-Instruct-75K", None, "train")]},
    {"name": "math", "target": N_MATH,
     "convert": from_prompt_response("problem", "generated_solution"),
     "streaming": True,
     "candidates": [("nvidia/OpenMathInstruct-2", None, "train")]},
    {"name": "general", "target": N_GENERAL, "convert": from_sharegpt,
     "streaming": True,
     "candidates": [("HuggingFaceH4/ultrachat_200k", None, "train_sft")]},
]

api = HfApi()
examples, seen_hashes = [], set()
manifest = {"sources": []}

for source in SOURCES:
    kept, scanned, remaining = 0, 0, source["target"]
    for repo, config, split in source["candidates"]:
        if remaining <= 0:
            break
        try:
            ds = load_dataset(repo, config, split=split,
                              streaming=source["streaming"])
        except Exception as err:
            print(f"[{source['name']}] SKIP {repo} ({config}): {err}")
            continue
        try:
            license_tag = getattr(api.dataset_info(repo).card_data, "license", None)
        except Exception:
            license_tag = "unknown"
        cand_kept = 0
        for row in ds:
            scanned += 1
            msgs = source["convert"](row)
            if msgs is None:
                pass
            else:
                sig = hashlib.sha256(
                    json.dumps(msgs, sort_keys=True).encode()).hexdigest()
                if sig not in seen_hashes:
                    seen_hashes.add(sig)
                    examples.append({"messages": msgs, "source": source["name"]})
                    kept += 1
                    cand_kept += 1
                    remaining -= 1
            if remaining <= 0 or scanned >= source["target"] * 5:
                break
        manifest["sources"].append({"repo": repo, "config": config,
                                    "license": license_tag, "kept": cand_kept})
        print(f"[{source['name']}] {repo} ({config}): kept {cand_kept} "
              f"(license: {license_tag})")
    status = "OK" if kept >= source["target"] * 0.5 else "*** LOW YIELD ***"
    print(f"[{source['name']}] total {kept}/{source['target']} {status}")

# Optional personal-specialization slice (see the markdown above).
personal_path = "/content/data/personal.jsonl"
if os.path.exists(personal_path):
    n_personal = 0
    with open(personal_path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                msgs = norm_messages(obj["messages"])
            except Exception:
                msgs = None
            if msgs:
                examples.append({"messages": msgs, "source": "personal"})
                n_personal += 1
    print(f"[personal] mixed in {n_personal} examples from personal.jsonl")

assert len(examples) >= 1000, (
    f"only {len(examples)} usable examples — dataset downloads mostly failed; "
    "check the warnings above before spending GPU time")

random.shuffle(examples)
with open(DATA_PATH, "w") as fh:
    for ex in examples:
        fh.write(json.dumps(ex, ensure_ascii=False) + "\n")
with open("/content/data/manifest.json", "w") as fh:
    json.dump(manifest, fh, indent=2)

from collections import Counter
print(f"\nwrote {len(examples)} examples to {DATA_PATH}")
print("mix:", dict(Counter(e['source'] for e in examples)))"""))

# ===========================================================================
cells.append(md("""## Step 6 — Mask-verification gate + tokenize + pack

The hard gate. Only the **tokenizer** is downloaded here (a few MB). On a sample of the real data it verifies: loss lands only inside assistant turns, never on tool/user/system text or `<tool_response>` content; the stop token is being taught; and segment-wise tokenization matches joint tokenization. **If this cell raises, do not train** — a bad mask trains the model to hallucinate tool output.

Then the full corpus is tokenized, overlong examples dropped (never truncated — cutting a trajectory mid-tool-call teaches malformed calls), split, and FFD-packed into full 16K rows."""))

cells.append(py(
"""from transformers import AutoTokenizer

from trainkit.data import (build_packed_dataset, iter_jsonl, load_and_tokenize,
                           rows_to_hf_columns, split_train_eval)
from trainkit.mask_check import verify_masking

tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, trust_remote_code=True)
if tokenizer.pad_token_id is None:
    tokenizer.pad_token = tokenizer.eos_token

# --- the gate ---------------------------------------------------------------
conversations = []
for _, obj in iter_jsonl(DATA_PATH):
    if isinstance(obj, dict) and isinstance(obj.get("messages"), list):
        conversations.append(obj["messages"])
    if len(conversations) >= 1000:
        break
report = verify_masking(conversations, tokenizer, sample_size=200)
print(report.summary())
assert report.passed, "MASK VERIFICATION FAILED — refusing to train on a bad mask."

# --- tokenize / split / pack -------------------------------------------------
examples, stats = load_and_tokenize(DATA_PATH, tokenizer, MAX_SEQ_LEN)
train_examples, eval_examples = split_train_eval(examples, EVAL_FRACTION, SEED)
train_rows, stats = build_packed_dataset(train_examples, MAX_SEQ_LEN, mode="ffd",
                                         shuffle_seed=SEED, stats=stats)
print(stats.summary())

from datasets import Dataset
train_ds = Dataset.from_dict(
    rows_to_hf_columns(train_rows, tokenizer.pad_token_id, MAX_SEQ_LEN))
eval_ds = None
if eval_examples:
    eval_rows, _ = build_packed_dataset(eval_examples, MAX_SEQ_LEN, mode="ffd",
                                        shuffle_seed=SEED)
    eval_ds = Dataset.from_dict(
        rows_to_hf_columns(eval_rows, tokenizer.pad_token_id, MAX_SEQ_LEN))

import math
steps_per_epoch = math.ceil(len(train_ds) / (BATCH_SIZE * GRAD_ACCUM))
total_steps = math.ceil(steps_per_epoch * NUM_EPOCHS)
print(f"\\ntrain rows: {len(train_ds)}"
      + (f", eval rows: {len(eval_ds)}" if eval_ds else "")
      + f"\\n~{steps_per_epoch} optimizer steps/epoch, ~{total_steps} total"
      + f" (a checkpoint every {SAVE_STEPS} steps)")"""))

# ===========================================================================
cells.append(md("""## Step 7 — Load the model and attach LoRA

Downloads the 27B base (~55GB — this is the long download) and attaches the LoRA adapter. **Read the coverage report this cell prints:** every target-module pattern should match a nonzero number of modules. Qwen3.6's hybrid Gated-DeltaNet blocks may expose extra projection names — if a pattern matches 0, or the printed module list shows obvious unwrapped projections, extend `TARGET_MODULES` in the CONFIGURATION cell and rerun from there."""))

cells.append(py(
"""import torch
from unsloth import FastLanguageModel

DTYPE = torch.bfloat16 if BF16_OK else torch.float16

model, _tok = FastLanguageModel.from_pretrained(
    model_name=MODEL_ID,
    max_seq_length=MAX_SEQ_LEN,
    dtype=DTYPE,
    load_in_4bit=LOAD_IN_4BIT,
    trust_remote_code=True,
)

model = FastLanguageModel.get_peft_model(
    model,
    r=LORA_R,
    lora_alpha=LORA_ALPHA,
    lora_dropout=0.0,
    target_modules=TARGET_MODULES,
    bias="none",
    use_gradient_checkpointing="unsloth",
    random_state=SEED,
)

# --- LoRA coverage report: a zero-match pattern means silently untrained layers
counts = {p: 0 for p in TARGET_MODULES}
for name, _ in model.named_modules():
    leaf = name.rsplit(".", 1)[-1]
    if leaf in counts and "lora" not in name.lower():
        counts[leaf] += 1
zero = [p for p, n in counts.items() if n == 0]
for p, n in counts.items():
    print(f"  {p:<12} matched {n} modules")
if zero:
    print(f"*** WARNING: {zero} matched NOTHING — inspect model.named_modules() "
          "and extend TARGET_MODULES for the hybrid DeltaNet blocks.")

# --- freeze the vision tower (unified multimodal checkpoint; we train text-only)
frozen = 0
for name, param in model.named_parameters():
    if any(k in name.lower() for k in ("visual", "vision_tower", "vision_model")):
        param.requires_grad_(False)
        frozen += 1
print(f"vision tower: froze {frozen} params"
      + (" (none found — text-only checkpoint?)" if frozen == 0 else ""))

trainable = sum(p.numel() for p in model.parameters() if p.requires_grad)
total = sum(p.numel() for p in model.parameters())
print(f"trainable: {trainable/1e6:.1f}M / {total/1e9:.1f}B "
      f"({100.0*trainable/total:.3f}%)")"""))

# ===========================================================================
cells.append(md("""## Step 8 — Train

Checkpoints every `SAVE_STEPS` optimizer steps to the output dir from Step 2. **Auto-resume:** if a complete checkpoint exists (training crashed, runtime restarted), rerunning this cell continues from it — it never restarts from scratch by accident."""))

cells.append(py(
"""import glob, os
from transformers import Trainer, TrainingArguments, default_data_collator

args = TrainingArguments(
    output_dir=OUTPUT_DIR,
    num_train_epochs=NUM_EPOCHS,
    per_device_train_batch_size=BATCH_SIZE,
    gradient_accumulation_steps=GRAD_ACCUM,
    learning_rate=LEARNING_RATE,
    lr_scheduler_type="cosine_with_min_lr",
    lr_scheduler_kwargs={"min_lr_rate": MIN_LR_RATIO},
    warmup_ratio=WARMUP_RATIO,
    weight_decay=WEIGHT_DECAY,
    max_grad_norm=1.0,
    optim="adamw_8bit",
    bf16=BF16_OK,
    fp16=not BF16_OK,
    logging_steps=LOGGING_STEPS,
    save_strategy="steps",
    save_steps=SAVE_STEPS,
    save_total_limit=SAVE_TOTAL_LIMIT,
    eval_strategy="steps" if eval_ds else "no",
    eval_steps=SAVE_STEPS if eval_ds else None,
    per_device_eval_batch_size=BATCH_SIZE,
    seed=SEED,
    report_to="none",
    remove_unused_columns=False,   # keep position_ids for the packed rows
)

trainer = Trainer(
    model=model,
    args=args,
    train_dataset=train_ds,
    eval_dataset=eval_ds,
    data_collator=default_data_collator,
)

# --- integrity-checked auto-resume ------------------------------------------
resume = None
ckpts = sorted(glob.glob(os.path.join(OUTPUT_DIR, "checkpoint-*")),
               key=lambda p: int(p.rsplit("-", 1)[1]))
for ckpt in reversed(ckpts):
    state_ok = os.path.getsize(os.path.join(ckpt, "trainer_state.json")) > 0 \\
        if os.path.exists(os.path.join(ckpt, "trainer_state.json")) else False
    weights_ok = any(os.path.exists(os.path.join(ckpt, w)) for w in
                     ("adapter_model.safetensors", "adapter_model.bin"))
    if state_ok and weights_ok:
        resume = ckpt
        break
    print(f"skipping incomplete checkpoint {ckpt}")
print(f"resume from: {resume or 'fresh start'}")

trainer.train(resume_from_checkpoint=resume)

os.makedirs(ADAPTER_DIR, exist_ok=True)
model.save_pretrained(ADAPTER_DIR)
tokenizer.save_pretrained(ADAPTER_DIR)
print(f"adapter saved to {ADAPTER_DIR}")
print("NOTE: the adapter (~1GB) is the one artifact worth getting off this VM "
      "immediately — download it or run the HF upload step. Everything "
      "downstream (merge, GGUF) can be re-derived from it.")"""))

# ===========================================================================
cells.append(md("## Step 9 — Generation smoke test\n\nQuick sanity check with the trained adapter still in VRAM: one planning prompt, one STEM prompt. Not a real evaluation (use the project's `evals/` suite for that) — just proof the artifact behaves before you spend an hour merging."))

cells.append(py(
"""FastLanguageModel.for_inference(model)

for user_prompt in [
    "Plan, step by step, how you would debug a Docker container that exits "
    "immediately with code 137, then give the first command you would run.",
    "A 2 kg mass hangs from a spring with k = 800 N/m. What is the natural "
    "frequency of oscillation in Hz?",
]:
    text = tokenizer.apply_chat_template(
        [{"role": "user", "content": user_prompt}],
        tokenize=False, add_generation_prompt=True)
    ids = tokenizer(text, return_tensors="pt").to(model.device)
    out = model.generate(**ids, max_new_tokens=350, temperature=0.7, do_sample=True)
    print("=" * 80)
    print(tokenizer.decode(out[0][ids["input_ids"].shape[1]:],
                           skip_special_tokens=True))"""))

# ===========================================================================
cells.append(md("""## Step 10 — Merge LoRA → 16-bit checkpoint

Frees the training model from VRAM, then merges on **CPU** — the G4's 180GB of system RAM handles the 27B comfortably (the guard scales with `MODEL_SIZE_B` and stops cleanly on smaller runtimes). Disk math is guarded too: the merged output needs ~2.2GB per billion params free; after it is verified on disk, the base-model download cache is deleted to make room for GGUF export.

Skip this and Step 11 entirely if you only want the LoRA adapter (it's already saved)."""))

cells.append(py(
"""import gc, shutil

# Free VRAM/RAM from the training objects.
for _name in ("trainer", "model"):
    if _name in globals():
        del globals()[_name]
gc.collect()
torch.cuda.empty_cache()

merged_need_gb = MODEL_SIZE_B * 2.2
free_gb = shutil.disk_usage("/").free / 1024**3
assert free_gb > merged_need_gb + 3, (
    f"only {free_gb:.0f}GB free — the merge needs ~{merged_need_gb:.0f}GB. "
    "Delete old artifacts and rerun this cell.")
assert RAM_GB > MODEL_SIZE_B * 2.4, (
    f"{RAM_GB:.0f}GB system RAM is not enough to CPU-merge a {MODEL_SIZE_B}B "
    "model (~{:.0f}GB needed). Download the adapter and merge on a bigger "
    "machine with training/merge_and_export.py.".format(MODEL_SIZE_B * 2.4))

from peft import PeftModel
from transformers import AutoModelForCausalLM

print("loading base model on CPU (RAM, not VRAM) — this takes a while...")
base = AutoModelForCausalLM.from_pretrained(
    MODEL_ID, torch_dtype=DTYPE, device_map="cpu",
    trust_remote_code=True)
merged = PeftModel.from_pretrained(base, ADAPTER_DIR).merge_and_unload()
os.makedirs(MERGED_DIR, exist_ok=True)
merged.save_pretrained(MERGED_DIR, safe_serialization=True)
tokenizer.save_pretrained(MERGED_DIR)
del merged, base
gc.collect()

# Verify BEFORE deleting anything.
merged_gb = sum(f.stat().st_size for f in __import__("pathlib").Path(MERGED_DIR)
                .rglob("*") if f.is_file()) / 1024**3
has_weights = any(fn.endswith(".safetensors") for fn in os.listdir(MERGED_DIR))
assert has_weights and merged_gb > MODEL_SIZE_B, \
    "merge verification FAILED — not deleting anything"
print(f"merged model verified: {merged_gb:.1f} GB at {MERGED_DIR}")

# Reclaim the base-model hub cache.
cache = os.path.expanduser(
    "~/.cache/huggingface/hub/models--" + MODEL_ID.replace("/", "--"))
if os.path.isdir(cache):
    shutil.rmtree(cache)
    print(f"deleted base model cache, "
          f"{shutil.disk_usage('/').free / 1024**3:.0f}GB now free")"""))

# ===========================================================================
cells.append(md("""## Step 11 — GGUF export

Clones and builds llama.cpp, converts the merged model to an f16 GGUF intermediate, then quantizes to each format in `GGUF_QUANTS` — **one at a time, disk-guarded** (guards scale with `MODEL_SIZE_B`). Sizes for a 27B: Q4_K_M ≈ 17GB, Q6_K ≈ 24GB, Q8_0 ≈ 30GB, f16 intermediate ≈ 55GB.

Download the finished `.gguf` from the Colab file browser, or push it to the Hub in Step 12 — GGUFs never go to Drive."""))

cells.append(py(
"""import shutil, subprocess

if GGUF_QUANTS:
    if not os.path.isdir("/content/llama.cpp"):
        subprocess.run(["git", "clone", "--depth", "1",
                        "https://github.com/ggml-org/llama.cpp",
                        "/content/llama.cpp"], check=True)
    quantize_bin = "/content/llama.cpp/build/bin/llama-quantize"
    if not os.path.exists(quantize_bin):
        subprocess.run(["cmake", "-B", "/content/llama.cpp/build",
                        "-S", "/content/llama.cpp", "-DGGML_CUDA=OFF"], check=True)
        subprocess.run(["cmake", "--build", "/content/llama.cpp/build",
                        "--target", "llama-quantize", "-j"], check=True)

    os.makedirs(GGUF_DIR, exist_ok=True)
    f16_path = f"{GGUF_DIR}/model-f16.gguf"

    f16_need_gb = MODEL_SIZE_B * 2.1
    free_gb = shutil.disk_usage("/").free / 1024**3
    assert free_gb > f16_need_gb + 4, (
        f"only {free_gb:.0f}GB free — the f16 GGUF intermediate needs "
        f"~{f16_need_gb:.0f}GB. Free space and rerun.")
    if not os.path.exists(f16_path):
        subprocess.run(["python", "/content/llama.cpp/convert_hf_to_gguf.py",
                        MERGED_DIR, "--outfile", f16_path, "--outtype", "f16"],
                       check=True)

    for quant in GGUF_QUANTS:
        out_path = f"{GGUF_DIR}/model-{quant.lower()}.gguf"
        quant_need_gb = MODEL_SIZE_B * 1.2
        free_gb = shutil.disk_usage("/").free / 1024**3
        assert free_gb > quant_need_gb, (
            f"only {free_gb:.0f}GB free before {quant} — download+delete a "
            "finished quant, then rerun.")
        subprocess.run([quantize_bin, f16_path, out_path, quant], check=True)
        print(f"{out_path}: "
              f"{os.path.getsize(out_path) / 1024**3:.1f} GB")

    os.remove(f16_path)
    print("removed f16 intermediate")
    print("\\nfinished GGUFs:", os.listdir(GGUF_DIR))
else:
    print("GGUF_QUANTS is empty — skipping export")"""))

# ===========================================================================
cells.append(md("## Step 12 — Optional: upload to the Hugging Face Hub\n\nSet `HF_UPLOAD_REPO` (and `HF_TOKEN`, or run `huggingface_hub.login()` interactively) in the CONFIGURATION cell. Uploads the adapter and any finished GGUFs — the most practical way to get the big artifacts off the ephemeral Colab disk."))

cells.append(py(
"""if HF_UPLOAD_REPO:
    from huggingface_hub import HfApi
    api = HfApi()
    api.create_repo(HF_UPLOAD_REPO, private=True, exist_ok=True)
    api.upload_folder(folder_path=ADAPTER_DIR, repo_id=HF_UPLOAD_REPO,
                      path_in_repo="adapter")
    print(f"adapter uploaded to {HF_UPLOAD_REPO}/adapter")
    if os.path.isdir(GGUF_DIR):
        for fn in os.listdir(GGUF_DIR):
            api.upload_file(path_or_fileobj=os.path.join(GGUF_DIR, fn),
                            path_in_repo=f"gguf/{fn}", repo_id=HF_UPLOAD_REPO)
            print(f"uploaded gguf/{fn}")
else:
    print("HF_UPLOAD_REPO not set — skipping upload")"""))

# ===========================================================================
cells.append(md("""## Done — what you have and what's next

- **LoRA adapter** at `ADAPTER_DIR` (on Drive if mounted) — small, re-mergeable any time
- **Merged bf16 model** at `MERGED_DIR` (local disk — download or upload before the session ends)
- **GGUF quant(s)** at `GGUF_DIR` — run locally with Ollama / llama.cpp / Open WebUI

Next steps from the project repo:
- **Evaluate**: serve the model (Ollama or llama.cpp `--api`), run the base model and your fine-tune through `evals/run_evals.py`, and compare — that A/B diff is the real verdict on the fine-tune.
- **ORPO pass (optional v1.1)**: with 5–15k preference pairs, `training/train_orpo.py` sharpens plans-before-acting / correct-tool-schema / clarify-vs-guess behavior on top of the merged checkpoint."""))

# ===========================================================================

notebook = {
    "cells": cells,
    "metadata": {
        "accelerator": "GPU",
        "colab": {"provenance": [], "gpuType": "A100"},
        "kernelspec": {"display_name": "Python 3", "language": "python",
                       "name": "python3"},
        "language_info": {"name": "python"},
    },
    "nbformat": 4,
    "nbformat_minor": 5,
}


def main() -> None:
    # nbformat expects `source` as a string or list; strings are fine.
    OUT.write_text(json.dumps(notebook, indent=1, ensure_ascii=False),
                   encoding="utf-8")
    n_code = sum(1 for c in cells if c["cell_type"] == "code")
    print(f"wrote {OUT.name}: {len(cells)} cells ({n_code} code)")


if __name__ == "__main__":
    main()
