# Downloads CC0 models and textures from Poly Haven (https://polyhaven.com,
# all assets CC0) into assets/polyhaven. Safe to re-run: skips existing files.
#   powershell -ExecutionPolicy Bypass -File tools/fetch_assets.ps1
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Join-Path $PSScriptRoot '..\assets\polyhaven'

function Get-File($url, $path) {
    if (Test-Path $path) { return }
    New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
    Invoke-WebRequest -UseBasicParsing $url -OutFile $path -TimeoutSec 300
}

# Only game-sized models: Poly Haven's trees are 7-17 million triangles each.
$models = 'shrub_02', 'fern_02', 'tree_stump_01', 'dead_tree_trunk', 'boulder_01', 'rock_moss_set_01', 'rock_07'
foreach ($m in $models) {
    try {
        $files = Invoke-RestMethod "https://api.polyhaven.com/files/$m" -TimeoutSec 30
        $g = $files.gltf.'1k'.gltf
        Get-File $g.url (Join-Path $root "models\$m\$m.gltf")
        foreach ($inc in $g.include.PSObject.Properties) {
            Get-File $inc.Value.url (Join-Path $root "models\$m\$($inc.Name)")
        }
        "model   ok  $m"
    } catch { "model   FAIL $m : $($_.Exception.Message)" }
}

# Surface name -> candidate Poly Haven textures (first one that exists wins).
$textures = [ordered]@{
    dirt      = 'forrest_ground_01', 'brown_mud_leaves_01'
    rock      = 'rock_face', 'rocky_terrain_02'
    sand      = 'coast_sand_01', 'coast_sand_02'
    cobble    = 'cobblestone_floor_08', 'cobblestone_floor_01'
    brick     = 'red_brick_03', 'red_brick', 'brick_wall_001'
    stonewall = 'castle_wall_slates', 'stone_wall', 'medieval_blocks_02'
    planks    = 'wood_planks', 'wooden_planks', 'plank_flooring'
    shingles  = 'roof_tiles', 'clay_roof_tiles', 'roof_09'
    sandstone = 'sandstone_blocks_08', 'sandstone_brick_wall_01', 'castle_brick_07'
}
foreach ($t in $textures.Keys) {
    $done = $false
    foreach ($name in $textures[$t]) {
        try {
            $f = Invoke-RestMethod "https://api.polyhaven.com/files/$name" -TimeoutSec 30
            $dir = Join-Path $root "textures\$t"
            Get-File $f.Diffuse.'1k'.jpg.url (Join-Path $dir 'diffuse.jpg')
            Get-File $f.nor_gl.'1k'.jpg.url (Join-Path $dir 'normal.jpg')
            Get-File $f.Displacement.'1k'.png.url (Join-Path $dir 'height.png')
            Set-Content (Join-Path $dir 'SOURCE.txt') "https://polyhaven.com/a/$name (CC0)"
            "texture ok  $t <- $name"; $done = $true; break
        } catch { }
    }
    if (-not $done) { "texture FAIL $t" }
}

# Game-ready low-poly trees and bushes: flo-bit's low poly nature pack (CC0),
# https://github.com/flo-bit/low-poly-asset-packs
$floBit = 'https://raw.githubusercontent.com/flo-bit/low-poly-asset-packs/main/nature-pack/glb'
$floBitModels = @()
foreach ($n in 1..3) {
    $floBitModels += "common_tree_$n", "common_tree_${n}_autumn", "pine_tree_$n", "pine_tree_${n}_autumn", "pine_tree_${n}_snow", "dead_tree_$n", "common_tree_dead_$n"
}
$floBitModels += 'bush_1', 'bush_2', 'bush_3', 'bush_4', 'bush_5'
foreach ($m in $floBitModels) {
    try { Get-File "$floBit/$m.glb" (Join-Path $root "..\trees\$m.glb"); "tree    ok  $m" } catch { "tree    FAIL $m" }
}
