"""Rasterize ui_capture --headless meshes without a window, GPU, or desktop capture.
Requires Pillow and numpy. Input contains simulated UI only; output is a layout preview,
not proof of native font antialiasing, window focus, or physical device behaviour.
"""
import argparse
import json
from pathlib import Path
import numpy as np
from PIL import Image


def render(path):
    data = json.loads(path.read_text(encoding="utf-8"))
    scale = data["scale"]
    width, height = round(data["width"] * scale), round(data["height"] * scale)
    canvas = np.full((height, width, 4), 255.0, dtype=np.float32)
    textures = {}
    for delta in data["textures"]:
        w, h = delta["size"]
        pixels = np.array(delta["pixels"], dtype=np.float32).reshape(h, w, 4)
        if delta["pos"] is None:
            textures[delta["id"]] = pixels
        else:
            x, y = delta["pos"]
            textures[delta["id"]][y:y+h, x:x+w] = pixels
    for mesh in data["meshes"]:
        texture = textures[mesh["texture"]]
        th, tw = texture.shape[:2]
        vertices = mesh["vertices"]
        clip = np.array(mesh["clip"]) * scale
        for j in range(0, len(mesh["indices"]), 3):
            v = [vertices[i] for i in mesh["indices"][j:j+3]]
            p = np.array([r[:2] for r in v]) * scale
            lo = np.maximum(np.floor(p.min(0)), [max(0, clip[0]), max(0, clip[1])]).astype(int)
            hi = np.minimum(np.ceil(p.max(0)), [min(width, clip[2]), min(height, clip[3])]).astype(int)
            if np.any(hi <= lo):
                continue
            a, b, c = p
            den = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if abs(den) < 1e-8:
                continue
            x, y = np.meshgrid(np.arange(lo[0], hi[0])+0.5, np.arange(lo[1], hi[1])+0.5)
            w0 = ((b[1]-c[1])*(x-c[0])+(c[0]-b[0])*(y-c[1]))/den
            w1 = ((c[1]-a[1])*(x-c[0])+(a[0]-c[0])*(y-c[1]))/den
            w2 = 1-w0-w1
            mask = (w0 >= 0) & (w1 >= 0) & (w2 >= 0)
            if not mask.any():
                continue
            weights = np.stack([w0[mask], w1[mask], w2[mask]], axis=1)
            uv = weights @ np.array([r[2:4] for r in v])
            tx = np.clip(uv[:, 0]*tw-0.5, 0, tw-1)
            ty = np.clip(uv[:, 1]*th-0.5, 0, th-1)
            x0, y0 = tx.astype(int), ty.astype(int)
            x1, y1 = np.minimum(x0+1, tw-1), np.minimum(y0+1, th-1)
            fx, fy = (tx-x0)[:, None], (ty-y0)[:, None]
            tex = (texture[y0,x0]*(1-fx)+texture[y0,x1]*fx)*(1-fy)+(texture[y1,x0]*(1-fx)+texture[y1,x1]*fx)*fy
            color = weights @ np.array([r[4] for r in v])
            source = tex * color / 255.0
            region = canvas[lo[1]:hi[1], lo[0]:hi[0]]
            region[mask] = source + region[mask] * (1-source[:,3:4]/255.0)
    output = path.with_suffix('').with_suffix('.png')
    Image.fromarray(np.clip(canvas[:,:,:3], 0, 255).astype(np.uint8)).save(output)
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    paths = sorted(args.path.rglob("*.mesh.json")) if args.path.is_dir() else [args.path]
    for path in paths:
        render(path)
