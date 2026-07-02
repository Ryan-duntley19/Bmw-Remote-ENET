#!/usr/bin/env python3
"""Validate the generated Colab notebook without running it.

Checks:
  1. Notebook JSON parses and has the expected structure.
  2. Every code cell compiles as Python (magics stripped; %%writefile cell
     bodies compiled as the file they write).
  3. Embedded trainkit modules are byte-identical to the unit-tested files
     in training/trainkit/ — the whole point of generating instead of
     hand-copying.
  4. Config-cell names used by later cells are actually defined in the
     configuration or drive cells (catches renamed-knob drift).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NB = Path(__file__).resolve().parent / "qwen36_27b_finetune_colab.ipynb"
TRAINKIT = ROOT / "training" / "trainkit"

failures: list[str] = []


def check(cond: bool, message: str) -> None:
    if not cond:
        failures.append(message)


nb = json.loads(NB.read_text(encoding="utf-8"))
cells = nb["cells"]
code_cells = [c for c in cells if c["cell_type"] == "code"]
check(len(code_cells) >= 15, f"expected >=15 code cells, got {len(code_cells)}")


def cell_source(cell) -> str:
    src = cell["source"]
    return src if isinstance(src, str) else "".join(src)


# --- 2: everything compiles -------------------------------------------------
writefile_cells: dict[str, str] = {}
for i, cell in enumerate(code_cells):
    src = cell_source(cell)
    m = re.match(r"%%writefile\s+(\S+)\n", src)
    if m:
        body = src[m.end():]
        writefile_cells[m.group(1)] = body
        try:
            compile(body, m.group(1), "exec")
        except SyntaxError as err:
            check(False, f"embedded file {m.group(1)} has a syntax error: {err}")
        continue
    stripped = "\n".join(
        line for line in src.splitlines()
        if not line.lstrip().startswith(("%", "!")))
    try:
        compile(stripped, f"code_cell_{i}", "exec")
    except SyntaxError as err:
        check(False, f"code cell {i} has a syntax error: {err}")

# --- 3: embedded modules byte-identical to the tested sources ----------------
expected = ["chat_format.py", "masking.py", "packing.py", "data.py",
            "mask_check.py"]
for name in expected:
    key = f"trainkit/{name}"
    check(key in writefile_cells, f"missing embedded module {key}")
    if key in writefile_cells:
        source = (TRAINKIT / name).read_text(encoding="utf-8")
        check(writefile_cells[key] == source,
              f"{key} differs from training/trainkit/{name} — regenerate the "
              "notebook with colab/build_notebook.py")

# --- 4: config-name drift -----------------------------------------------------
all_code = "\n".join(cell_source(c) for c in code_cells)
defining = "\n".join(cell_source(c) for c in code_cells[:4])  # env/config/drive/install
for knob in ("MODEL_ID", "MODEL_SIZE_B", "AUTO_ADJUST", "GPU_VRAM_GB", "RAM_GB",
             "BF16_OK", "CKPT_EST_GB",
             "MAX_SEQ_LEN", "LOAD_IN_4BIT", "LORA_R", "LORA_ALPHA",
             "TARGET_MODULES", "LEARNING_RATE", "NUM_EPOCHS", "BATCH_SIZE",
             "GRAD_ACCUM", "WARMUP_RATIO", "WEIGHT_DECAY", "MIN_LR_RATIO",
             "SEED", "SAVE_STEPS", "SAVE_TOTAL_LIMIT", "LOGGING_STEPS",
             "USE_DRIVE", "N_TOOL_CALLING", "N_CODE", "N_MATH", "N_GENERAL",
             "EVAL_FRACTION", "GGUF_QUANTS", "HF_TOKEN", "HF_UPLOAD_REPO",
             "OUTPUT_DIR", "ADAPTER_DIR", "MERGED_DIR", "GGUF_DIR", "DATA_PATH"):
    check(re.search(rf"^\s*{knob}\s*=", defining, re.MULTILINE) is not None,
          f"config name {knob} used later but not defined in the config/drive cells")
    check(knob in all_code, f"config name {knob} defined but never used")

# --- verdict ------------------------------------------------------------------
if failures:
    print("VALIDATION FAILED:")
    for f in failures:
        print(f"  - {f}")
    sys.exit(1)
print(f"notebook OK: {len(cells)} cells, {len(code_cells)} code, "
      f"{len(writefile_cells)} embedded modules verified identical to trainkit/")
