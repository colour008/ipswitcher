param(
    [string]$OutPng
)

Add-Type -AssemblyName System.Drawing

$size = 1024
$bmp = New-Object System.Drawing.Bitmap $size, $size
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias

# Rounded badge path
$rect = New-Object System.Drawing.Rectangle 36, 36, 952, 952
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$r = 190
$path.AddArc($rect.X, $rect.Y, $r, $r, 180, 90)
$path.AddArc($rect.Right - $r, $rect.Y, $r, $r, 90, 90)
$path.AddArc($rect.Right - $r, $rect.Bottom - $r, $r, $r, 0, 90)
$path.AddArc($rect.X, $rect.Bottom - $r, $r, $r, 270, 90)
$path.CloseFigure()

$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(37, 99, 235),
    [System.Drawing.Color]::FromArgb(30, 58, 138),
    45
)
$g.FillPath($brush, $path)

# Network node links
$pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(150, 255, 255, 255)), 7
$nodes = @(
    @(250, 270),
    @(774, 250),
    @(260, 750),
    @(770, 760)
)
$g.DrawLine($pen, $nodes[0][0], $nodes[0][1], $nodes[1][0], $nodes[1][1])
$g.DrawLine($pen, $nodes[1][0], $nodes[1][1], $nodes[3][0], $nodes[3][1])
$g.DrawLine($pen, $nodes[3][0], $nodes[3][1], $nodes[2][0], $nodes[2][1])
$g.DrawLine($pen, $nodes[2][0], $nodes[2][1], $nodes[0][0], $nodes[0][1])

$nodeBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
foreach ($n in $nodes) {
    $g.FillEllipse($nodeBrush, $n[0] - 26, $n[1] - 26, 52, 52)
}

# Main label IP
$font = New-Object System.Drawing.Font("Arial", 300, [System.Drawing.FontStyle]::Bold)
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$sf.LineAlignment = [System.Drawing.StringAlignment]::Center
$textBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
$textRect = New-Object System.Drawing.RectangleF 0, 130, 1024, 764
$g.DrawString("IP", $font, $textBrush, $textRect, $sf)

$bmp.Save($OutPng, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
Write-Host "icon saved: $OutPng"
