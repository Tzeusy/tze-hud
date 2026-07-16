$ErrorActionPreference = "Stop"

$harnessPath = Join-Path $PSScriptRoot "windowed-fullscreen-overlay-perf.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $harnessPath,
    [ref]$tokens,
    [ref]$parseErrors
)
if ($parseErrors.Count -gt 0) {
    throw "Harness has PowerShell parse errors: $($parseErrors -join '; ')"
}

foreach ($functionName in @("Get-EffectiveSurfaceDimensions", "Assert-ComparableEffectiveSurfaces")) {
    $definition = $ast.Find(
        {
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -eq $functionName
        },
        $true
    )
    if ($null -eq $definition) {
        throw "Harness function not found: $functionName"
    }
    Invoke-Expression $definition.Extent.Text
}

function New-BenchmarkArtifact {
    param(
        [Parameter(Mandatory=$true)]
        [object]$Width,

        [Parameter(Mandatory=$true)]
        [object]$Height
    )

    return [pscustomobject]@{
        window = [pscustomobject]@{
            width = $Width
            height = $Height
        }
    }
}

function Assert-ThrowsMatching {
    param(
        [Parameter(Mandatory=$true)]
        [scriptblock]$Action,

        [Parameter(Mandatory=$true)]
        [string]$Pattern
    )

    $threw = $false
    try {
        & $Action | Out-Null
    } catch {
        $threw = $true
        if ($_.Exception.Message -notmatch $Pattern) {
            throw "Expected error matching '$Pattern', got '$($_.Exception.Message)'"
        }
    }
    if (-not $threw) {
        throw "Expected error matching '$Pattern', but the action succeeded"
    }
}

$validFullscreen = New-BenchmarkArtifact -Width 3840 -Height 2160
$validOverlay = New-BenchmarkArtifact -Width 3840 -Height 2160
$surface = Assert-ComparableEffectiveSurfaces `
    -FullscreenArtifact $validFullscreen `
    -OverlayArtifact $validOverlay
if ($surface.width -ne 3840 -or $surface.height -ne 2160) {
    throw "Equal effective dimensions were not preserved"
}

Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact ([pscustomobject]@{}) -OverlayArtifact $validOverlay } `
    -Pattern "missing window object"

$missingWidth = [pscustomobject]@{ window = [pscustomobject]@{ height = 2160 } }
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $missingWidth -OverlayArtifact $validOverlay } `
    -Pattern "missing window.width"

$missingHeight = [pscustomobject]@{ window = [pscustomobject]@{ width = 3840 } }
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $missingHeight -OverlayArtifact $validOverlay } `
    -Pattern "missing window.height"

$malformedWidth = New-BenchmarkArtifact -Width "not-an-integer" -Height 2160
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $malformedWidth -OverlayArtifact $validOverlay } `
    -Pattern "malformed window.width"

$stringWidth = New-BenchmarkArtifact -Width "3840" -Height 2160
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $stringWidth -OverlayArtifact $validOverlay } `
    -Pattern "malformed window.width"

$malformedHeight = New-BenchmarkArtifact -Width 3840 -Height 1080.5
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $malformedHeight -OverlayArtifact $validOverlay } `
    -Pattern "malformed window.height"

$zeroWidth = New-BenchmarkArtifact -Width 0 -Height 2160
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $zeroWidth -OverlayArtifact $validOverlay } `
    -Pattern "malformed window.width"

$negativeHeight = New-BenchmarkArtifact -Width 3840 -Height (-1)
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $negativeHeight -OverlayArtifact $validOverlay } `
    -Pattern "malformed window.height"

$unequalOverlay = New-BenchmarkArtifact -Width 1920 -Height 1080
Assert-ThrowsMatching `
    -Action { Assert-ComparableEffectiveSurfaces -FullscreenArtifact $validFullscreen -OverlayArtifact $unequalOverlay } `
    -Pattern "effective surface mismatch"

Write-Host "windowed fullscreen/overlay harness contract tests passed"
