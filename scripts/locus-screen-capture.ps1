param(
  [Parameter(Mandatory = $true)][string]$OutputPath
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

$screens = [System.Windows.Forms.Screen]::AllScreens
$left = ($screens | ForEach-Object { $_.Bounds.Left } | Measure-Object -Minimum).Minimum
$top = ($screens | ForEach-Object { $_.Bounds.Top } | Measure-Object -Minimum).Minimum
$right = ($screens | ForEach-Object { $_.Bounds.Right } | Measure-Object -Maximum).Maximum
$bottom = ($screens | ForEach-Object { $_.Bounds.Bottom } | Measure-Object -Maximum).Maximum
$width = $right - $left
$height = $bottom - $top
$bitmap = [System.Drawing.Bitmap]::new([int]$width, [int]$height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
  $graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
  $directory = [System.IO.Path]::GetDirectoryName([System.IO.Path]::GetFullPath($OutputPath))
  [System.IO.Directory]::CreateDirectory($directory) | Out-Null
  $bitmap.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
  $graphics.Dispose()
  $bitmap.Dispose()
}

[ordered]@{ path = [System.IO.Path]::GetFullPath($OutputPath); left = $left; top = $top; width = $width; height = $height } |
  ConvertTo-Json -Compress
