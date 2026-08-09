#!/usr/bin/env python3
"""Data-oriented texture statistics for Locus graphics-debugger exports."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path
from typing import Any

__all__ = ["analyze_texture", "compare_textures"]


def _dependencies():
    try:
        import numpy as np
        from PIL import Image
    except ImportError as exc:
        raise RuntimeError(
            "Texture analysis requires numpy and Pillow in the active Python runtime."
        ) from exc
    return np, Image


def _open(path: Path, crop: tuple[int, int, int, int] | None):
    np, Image = _dependencies()
    try:
        image = Image.open(path)
        image.load()
    except Exception as exc:
        if path.suffix.lower() == ".exr":
            raise RuntimeError(
                "The active Pillow runtime cannot decode this EXR. Install an EXR-capable "
                "Pillow/OpenImageIO codec or export PNG for normalized analysis."
            ) from exc
        raise

    source_format = image.format or path.suffix.lstrip(".").upper()
    source_mode = image.mode
    if crop is not None:
        x, y, width, height = crop
        if width <= 0 or height <= 0:
            raise ValueError("Crop width and height must be positive.")
        image = image.crop((x, y, x + width, y + height))

    if image.mode not in {"L", "LA", "RGB", "RGBA", "I", "F", "I;16"}:
        image = image.convert("RGBA")
    array = np.asarray(image)
    if array.ndim == 2:
        array = array[..., np.newaxis]
    return image, array, source_format, source_mode


def _normalized(array):
    np, _ = _dependencies()
    if np.issubdtype(array.dtype, np.floating):
        return array.astype(np.float64, copy=False), None
    info = np.iinfo(array.dtype)
    scale = float(info.max)
    return array.astype(np.float64) / scale, scale


def _channel_names(mode: str, count: int) -> list[str]:
    known = {
        "L": ["l"],
        "LA": ["l", "a"],
        "RGB": ["r", "g", "b"],
        "RGBA": ["r", "g", "b", "a"],
    }
    names = known.get(mode)
    if names is not None and len(names) == count:
        return names
    return [f"c{index}" for index in range(count)]


def _round(value: Any, digits: int = 6):
    np, _ = _dependencies()
    if isinstance(value, (np.integer, int)):
        return int(value)
    result = float(value)
    if not math.isfinite(result):
        return None
    return round(result, digits)


def _stats(values) -> dict[str, Any]:
    np, _ = _dependencies()
    percentiles = np.percentile(values, [1, 5, 50, 95, 99])
    return {
        "min": _round(np.min(values)),
        "max": _round(np.max(values)),
        "mean": _round(np.mean(values)),
        "std": _round(np.std(values)),
        "p01": _round(percentiles[0]),
        "p05": _round(percentiles[1]),
        "p50": _round(percentiles[2]),
        "p95": _round(percentiles[3]),
        "p99": _round(percentiles[4]),
    }


def _luminance(normalized, names: list[str]):
    np, _ = _dependencies()
    if all(channel in names for channel in ("r", "g", "b")):
        red = normalized[..., names.index("r")]
        green = normalized[..., names.index("g")]
        blue = normalized[..., names.index("b")]
        return 0.2126 * red + 0.7152 * green + 0.0722 * blue
    return normalized[..., 0]


def _entropy(values, bins: int) -> float:
    np, _ = _dependencies()
    finite = values[np.isfinite(values)]
    if finite.size == 0:
        return 0.0
    low = float(np.min(finite))
    high = float(np.max(finite))
    if high <= low:
        return 0.0
    histogram, _ = np.histogram(finite, bins=bins, range=(low, high))
    probabilities = histogram[histogram > 0].astype(np.float64)
    probabilities /= probabilities.sum()
    return float(-np.sum(probabilities * np.log2(probabilities)))


def analyze(path: Path, crop, bins: int) -> tuple[dict[str, Any], Any]:
    np, _ = _dependencies()
    image, array, source_format, source_mode = _open(path, crop)
    normalized, integer_scale = _normalized(array)
    names = _channel_names(image.mode, normalized.shape[2])
    luminance = _luminance(normalized, names)

    channels: dict[str, Any] = {}
    for index, name in enumerate(names):
        values = normalized[..., index]
        channel = _stats(values)
        channel["lowClipRatio"] = _round(np.mean(values <= 1.0 / 255.0))
        channel["highClipRatio"] = _round(np.mean(values >= 254.0 / 255.0))
        channels[name] = channel

    horizontal = np.abs(np.diff(luminance, axis=1)) if luminance.shape[1] > 1 else np.zeros((0,))
    vertical = np.abs(np.diff(luminance, axis=0)) if luminance.shape[0] > 1 else np.zeros((0,))
    edge_samples = horizontal.size + vertical.size
    edge_count = int(np.count_nonzero(horizontal > 0.1) + np.count_nonzero(vertical > 0.1))

    alpha = normalized[..., names.index("a")] if "a" in names else None
    alpha_stats = None
    if alpha is not None:
        alpha_stats = {
            **_stats(alpha),
            "transparentRatio": _round(np.mean(alpha <= 1.0 / 255.0)),
            "translucentRatio": _round(
                np.mean((alpha > 1.0 / 255.0) & (alpha < 254.0 / 255.0))
            ),
            "opaqueRatio": _round(np.mean(alpha >= 254.0 / 255.0)),
        }

    flattened = normalized.reshape(-1, normalized.shape[2])
    if flattened.shape[0] > 250_000:
        step = max(1, flattened.shape[0] // 250_000)
        unique_sample = flattened[::step]
        unique_is_sampled = True
    else:
        unique_sample = flattened
        unique_is_sampled = False
    quantized = np.clip(np.rint(unique_sample * 255.0), 0, 255).astype(np.uint8)
    unique_colors = int(np.unique(quantized, axis=0).shape[0])

    result: dict[str, Any] = {
        "path": str(path.resolve()).replace("\\", "/"),
        "format": source_format,
        "sourceMode": source_mode,
        "mode": image.mode,
        "width": image.width,
        "height": image.height,
        "pixels": image.width * image.height,
        "dtype": str(array.dtype),
        "integerScale": integer_scale,
        "channels": channels,
        "luminance": {
            **_stats(luminance),
            "entropyBits": _round(_entropy(luminance, bins)),
            "edgeDensity": _round(edge_count / edge_samples if edge_samples else 0.0),
        },
        "uniqueColors8Bit": unique_colors,
        "uniqueColorsSampled": unique_is_sampled,
    }
    if alpha_stats is not None:
        result["alpha"] = alpha_stats
    return result, normalized


def compare(left, right) -> dict[str, Any]:
    np, _ = _dependencies()
    if left.shape != right.shape:
        raise ValueError(
            f"Compared textures must have the same shape: {left.shape} != {right.shape}."
        )
    difference = np.abs(left - right)
    squared = np.square(left - right)
    mae = float(np.mean(difference))
    mse = float(np.mean(squared))
    rmse = math.sqrt(mse)
    psnr = None if mse == 0 else 10.0 * math.log10(1.0 / mse)
    pixel_difference = np.max(difference, axis=2)
    return {
        "mae": _round(mae),
        "rmse": _round(rmse),
        "psnrDb": _round(psnr) if psnr is not None else None,
        "maxAbsoluteError": _round(np.max(difference)),
        "changedPixelRatio": _round(np.mean(pixel_difference > 1.0 / 255.0)),
        "changedPixelRatio1Percent": _round(np.mean(pixel_difference > 0.01)),
        "identical": bool(np.array_equal(left, right)),
    }


def analyze_texture(
    path: str | Path,
    crop: tuple[int, int, int, int] | None = None,
    histogram_bins: int = 256,
) -> dict[str, Any]:
    """Analyze one texture and return ordinary Python data types."""
    source = Path(path)
    if not source.is_file():
        raise FileNotFoundError(source)
    bins = max(16, min(4096, int(histogram_bins)))
    report, _ = analyze(source, crop, bins)
    return report


def compare_textures(
    left_path: str | Path,
    right_path: str | Path,
    crop: tuple[int, int, int, int] | None = None,
    histogram_bins: int = 256,
) -> dict[str, Any]:
    """Analyze and compare two same-shaped textures."""
    left_source = Path(left_path)
    right_source = Path(right_path)
    if not left_source.is_file():
        raise FileNotFoundError(left_source)
    if not right_source.is_file():
        raise FileNotFoundError(right_source)
    bins = max(16, min(4096, int(histogram_bins)))
    left_report, left = analyze(left_source, crop, bins)
    right_report, right = analyze(right_source, crop, bins)
    return {
        "left": left_report,
        "right": right_report,
        "comparison": compare(left, right),
    }


def parse_crop(value: str | None):
    if value is None:
        return None
    parts = [int(part.strip()) for part in value.split(",")]
    if len(parts) != 4:
        raise argparse.ArgumentTypeError("--crop must be x,y,width,height")
    return tuple(parts)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--crop", help="x,y,width,height")
    parser.add_argument("--histogram-bins", type=int, default=256)
    args = parser.parse_args()

    try:
        crop = parse_crop(args.crop)
        if not args.input.is_file():
            raise FileNotFoundError(args.input)
        bins = max(16, min(4096, args.histogram_bins))
        analysis, normalized = analyze(args.input, crop, bins)
        if args.compare is not None:
            if not args.compare.is_file():
                raise FileNotFoundError(args.compare)
            compared, compared_normalized = analyze(args.compare, crop, bins)
            analysis["comparison"] = {
                "path": compared["path"],
                **compare(normalized, compared_normalized),
            }
        payload = json.dumps(analysis, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
        if args.output is not None:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(payload + "\n", encoding="utf-8")
        print(payload)
        return 0
    except Exception as exc:
        print(json.dumps({"error": str(exc)}, ensure_ascii=False, separators=(",", ":")), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
