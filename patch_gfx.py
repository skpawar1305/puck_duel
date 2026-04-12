import re

with open('frontend-src/lib/components/Game.svelte', 'r') as f:
    text = f.read()

# Introduce cache flag
cache_var = "\n  let cacheWallFlash = -1;\n"
if "let cacheWallFlash" not in text:
    text = text.replace("let tableFxEnabled = true;", "let tableFxEnabled = true;\n  let cacheWallFlash = -1;")

# Update the loop to check it
update_block = """    if (tableFxEnabled) {
      try {
        if (rs.wall_flash !== cacheWallFlash) {
          cacheWallFlash = rs.wall_flash;
          drawBorderGlow(borderGlowGfx, rs.wall_flash);
          drawBorderRunners(borderRunnerGfx, rs.wall_flash);
          drawMidlineGlow(midlineGlowGfx, rs.wall_flash);
          drawWallFlashOverlay(wallFlashGfx, rs.wall_flash);
          drawBorder(borderGfx, rs.wall_flash);
          drawGoals(goalsGfx, rs.wall_flash);
          drawMidline(midlineGfx, rs.wall_flash);
        }
        drawTrailGfx(trailGfx, trail);
      } catch (e) {"""
text = re.sub(
    r"""    if \(tableFxEnabled\) \{\s*try \{\s*drawBorderGlow.*?drawTrailGfx\(trailGfx, trail\);\s*\} catch \(e\) \{""",
    update_block,
    text,
    flags=re.MULTILINE | re.DOTALL
)

# Strip out performance.now() ripples so rendering math matches cache logic safely
text = re.sub(
    r"const idle = 0.06 \+ \(Math.sin\(performance.now\(\) \* [0-9.]+\) \+ 1\) \* 0.025;",
    "const idle = 0.085;", text
)
text = re.sub(
    r"const pulse = 0.05 \+ \(Math.sin\(performance.now\(\) \* [0-9.]+\) \+ 1\) \* 0.05;",
    "const pulse = 0.1;", text
)
text = re.sub(
    r"const pulse = 0.08 \+ \(Math.sin\(performance.now\(\) \* [0-9.]+\) \+ 1\) \* 0.05;",
    "const pulse = 0.13;", text
)
text = re.sub(
    r"const wave = 0.1 \+ \(Math.sin\(performance.now\(\) \* [0-9.]+\) \+ 1\) \* 0.08;",
    "const wave = 0.18;", text
)

# For drawBorder, line 377: const t = performance.now() * 0.004; -> remove it
text = re.sub(r"const t \= performance\.now\(\) \* 0\.004;\n", "", text)

# For drawBorderRunners, drop the runner animation since t varies rapidly 
text = re.sub(
    r"const t \= performance\.now\(\) \* 0\.0038;\n.*?const xTopRight.*?;\n",
    """const xTopLeft = CR + 0.5 * topLeftSpan;
    const xTopRight = GX + GOAL_W + 0.5 * topRightSpan;
""", text, flags=re.MULTILINE | re.DOTALL)

with open('frontend-src/lib/components/Game.svelte', 'w') as f:
    f.write(text)
print("Graphics caching applied successfully.")
