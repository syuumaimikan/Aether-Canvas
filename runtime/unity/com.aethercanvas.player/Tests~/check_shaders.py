#!/usr/bin/env python3
"""Compile the HLSL inside the package's ShaderLab files with glslang.

Unity itself is needed to compile ShaderLab, but the programs inside are
plain HLSL. This pulls out each pass's CGINCLUDE + CGPROGRAM code, stands
in for the little of UnityCG.cginc it uses, and compiles every vertex and
fragment entry point (both colour-space variants), so a typo or type error
fails the tests rather than a user's import. Run by runtime/unity/test.sh.
"""

import pathlib
import re
import subprocess
import sys
import tempfile

PRELUDE = """
#define fixed half
#define fixed3 half3
#define fixed4 half4
float4x4 unity_ObjectToWorld;
float4x4 unity_MatrixVP;
float4 UnityObjectToClipPos(float4 v) { return mul(unity_MatrixVP, mul(unity_ObjectToWorld, float4(v.xyz, 1.0))); }
"""


def programs(source):
    include = "".join(re.findall(r"CGINCLUDE(.*?)ENDCG", source, re.S))
    for body in re.findall(r"CGPROGRAM(.*?)ENDCG", source, re.S):
        vertex = re.search(r"#pragma\s+vertex\s+(\w+)", body).group(1)
        fragment = re.search(r"#pragma\s+fragment\s+(\w+)", body).group(1)
        code = re.sub(r"#pragma[^\n]*", "", include + body)
        code = code.replace('#include "UnityCG.cginc"', "")
        yield vertex, fragment, PRELUDE + code


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    failures = 0
    checked = 0
    for shader in sorted(root.glob("Runtime/**/*.shader")):
        for n, (vertex, fragment, code) in enumerate(programs(shader.read_text())):
            for variant in ([], ["-DUNITY_COLORSPACE_GAMMA"]):
                for stage, entry in (("vert", vertex), ("frag", fragment)):
                    with tempfile.NamedTemporaryFile("w", suffix=".hlsl", delete=False) as f:
                        f.write(code)
                    result = subprocess.run(
                        ["glslangValidator", "-D", "-V", "--hlsl-dx9-compatible", "--auto-map-bindings",
                         "--auto-map-locations", "-S", stage, "-e", entry, *variant, "-o", "/dev/null", f.name],
                        capture_output=True,
                        text=True,
                    )
                    pathlib.Path(f.name).unlink()
                    checked += 1
                    what = f"{shader.name} pass {n} {stage} {entry} {' '.join(variant)}".strip()
                    if result.returncode:
                        failures += 1
                        print(f"not ok - {what}\n{result.stdout}{result.stderr}")
                    else:
                        print(f"ok - {what}")
    print(f"{checked} compiled, {failures} failed")
    sys.exit(1 if failures or not checked else 0)


if __name__ == "__main__":
    main()
