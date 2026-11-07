# DriverUI.ps1 - window, theme and states for DriverScan.ps1 (dot-sourced; shares its $script: state).
# Presentation only: the buttons and the timer call Invoke-* functions in DriverScan.ps1.
# Flow on screen: scan -> review the list inline -> install (live per-driver status) -> results with next steps.

function Rgb($r, $g, $b) { [Drawing.Color]::FromArgb($r, $g, $b) }
function New-Font($size, [bool]$bold = $false) { [Drawing.Font]::new('Segoe UI', [single]$size, $(if ($bold) { [Drawing.FontStyle]::Bold } else { [Drawing.FontStyle]::Regular })) }

# Text colors are checked >= 4.5:1 against White, Page and every *Soft tint they can sit on (incl. selected rows). Disabled is exempt.
$script:Theme = @{
    Navy = (Rgb 15 37 71); Page = (Rgb 244 246 249); Border = (Rgb 218 223 231)
    Text = (Rgb 27 39 51); Muted = (Rgb 78 90 110); OnNavy = (Rgb 190 206 235)
    Accent = (Rgb 37 99 235); AccentDark = (Rgb 29 78 216); AccentSoft = (Rgb 234 241 255)
    Good = (Rgb 17 116 54); GoodSoft = (Rgb 232 245 236)
    Warn = (Rgb 158 70 6); WarnSoft = (Rgb 254 243 226)
    Bad = (Rgb 185 28 28); BadSoft = (Rgb 253 236 236)
    Disabled = (Rgb 226 232 240); DisabledText = (Rgb 100 116 139)
}
$script:isBusy = $false
$script:scanned = $false
$script:syncing = $false
$script:gridMode = 'review'      # review | run | results
$script:IconCache = @{}

# ---------- drawn icons (one consistent round-cap stroke; no emoji or font glyphs) ----------
function New-Icon($kind, $color, [int]$size = 16) {
    $key = "$kind|$($color.ToArgb())|$size"
    if ($script:IconCache.ContainsKey($key)) { return $script:IconCache[$key] }
    $bmp = New-Object Drawing.Bitmap($size, $size)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'; $g.Clear([Drawing.Color]::Transparent)
    $pen = New-Object Drawing.Pen($color, [single]($size / 8.0)); $pen.StartCap = 'Round'; $pen.EndCap = 'Round'; $pen.LineJoin = 'Round'
    $brush = New-Object Drawing.SolidBrush($color)
    $s = [single]$size
    switch ($kind) {
        'check' { $g.DrawLines($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .18), ($s * .54))), (New-Object Drawing.PointF(($s * .42), ($s * .78))), (New-Object Drawing.PointF(($s * .84), ($s * .26))))) }
        'cross' { $g.DrawLine($pen, ($s * .24), ($s * .24), ($s * .76), ($s * .76)); $g.DrawLine($pen, ($s * .76), ($s * .24), ($s * .24), ($s * .76)) }
        'warn'  {
            $g.DrawPolygon($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .5), ($s * .12))), (New-Object Drawing.PointF(($s * .92), ($s * .84))), (New-Object Drawing.PointF(($s * .08), ($s * .84)))))
            $g.DrawLine($pen, ($s * .5), ($s * .4), ($s * .5), ($s * .6)); $g.FillEllipse($brush, ($s * .45), ($s * .68), ($s * .1), ($s * .1))
        }
        'shield' {
            $g.DrawPolygon($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .5), ($s * .1))), (New-Object Drawing.PointF(($s * .84), ($s * .24))), (New-Object Drawing.PointF(($s * .84), ($s * .52))), (New-Object Drawing.PointF(($s * .5), ($s * .9))), (New-Object Drawing.PointF(($s * .16), ($s * .52))), (New-Object Drawing.PointF(($s * .16), ($s * .24)))))
            $g.DrawLines($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .36), ($s * .5))), (New-Object Drawing.PointF(($s * .46), ($s * .6))), (New-Object Drawing.PointF(($s * .65), ($s * .38)))))
        }
        'clock' { $g.DrawEllipse($pen, ($s * .12), ($s * .12), ($s * .76), ($s * .76)); $g.DrawLines($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .5), ($s * .3))), (New-Object Drawing.PointF(($s * .5), ($s * .5))), (New-Object Drawing.PointF(($s * .66), ($s * .6))))) }
        'minus' { $g.DrawEllipse($pen, ($s * .12), ($s * .12), ($s * .76), ($s * .76)); $g.DrawLine($pen, ($s * .32), ($s * .5), ($s * .68), ($s * .5)) }
        'info'  { $g.DrawEllipse($pen, ($s * .12), ($s * .12), ($s * .76), ($s * .76)); $g.DrawLine($pen, ($s * .5), ($s * .46), ($s * .5), ($s * .69)); $g.FillEllipse($brush, ($s * .445), ($s * .27), ($s * .11), ($s * .11)) }
        'wait'   { $g.DrawEllipse($pen, ($s * .2), ($s * .2), ($s * .6), ($s * .6)) }
        'active' { $g.FillEllipse($brush, ($s * .22), ($s * .22), ($s * .56), ($s * .56)) }
        'up'     { $g.DrawLine($pen, ($s * .5), ($s * .84), ($s * .5), ($s * .2)); $g.DrawLines($pen, [Drawing.PointF[]]@((New-Object Drawing.PointF(($s * .24), ($s * .46))), (New-Object Drawing.PointF(($s * .5), ($s * .18))), (New-Object Drawing.PointF(($s * .76), ($s * .46))))) }
    }
    $g.Dispose(); $pen.Dispose(); $brush.Dispose()
    $script:IconCache[$key] = $bmp
    return $bmp
}

# ---------- plain-language labels ----------
function Get-ClassLabel($c) {
    $map = @{ DISPLAY = 'Graphics'; NET = 'Network'; MEDIA = 'Audio'; BLUETOOTH = 'Bluetooth'; SYSTEM = 'System / Chipset'; USB = 'USB'; HDC = 'Storage'; SCSIADAPTER = 'Storage'; CAMERA = 'Camera'; IMAGE = 'Camera / Scanner'; MONITOR = 'Monitor'; HIDCLASS = 'Input device'; KEYBOARD = 'Keyboard'; MOUSE = 'Mouse'; PRINTER = 'Printer'; FIRMWARE = 'Firmware'; PROCESSOR = 'Processor' }
    if ($c -and $map.ContainsKey([string]$c)) { return $map[[string]$c] }
    return [string]$c
}
function Get-SourceLabel($s) { if ($s -eq 'Catalog') { 'Your catalog' } else { [string]$s } }
function Format-SizeText($mb) {
    if ($null -eq $mb -or $mb -eq '') { return '' }
    $v = [double]$mb
    if ($v -le 0) { return '' }
    if ($v -lt 1) { return ('{0:N0} KB' -f ($v * 1024)) }
    if ($v -ge 1024) { return ('{0:N1} GB' -f ($v / 1024)) }
    return ('{0:N1} MB' -f $v)
}
# Intel INF drivers carry a fake 1968 date: show a dash instead of a misleading date.
function Format-DateText($d) {
    if (-not $d) { return '' }
    try { $x = [datetime]$d; if ($x.Year -lt 1990) { return '—' }; return $x.ToString('yyyy-MM-dd') } catch { return [string]$d }
}

# ---------- controls ----------
# Rounded, owner-drawn button (WinForms has no corner radius). Kinds: primary (solid + soft shadow), secondary (white, light border), link (text only).
# Colors come from $script:Theme. The 2px margin around the fill holds the keyboard focus ring, so focus never shifts the layout.
if (-not ('TierButton' -as [type])) {
    Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing -TypeDefinition @"
using System; using System.Drawing; using System.Drawing.Drawing2D; using System.Windows.Forms;
public class TierButton : Button {
    public static Color Accent, AccentDark, AccentSoft, Disabled, DisabledText, Ring, Edge;
    public string Kind = "secondary";
    bool hover, down;
    public TierButton() {
        SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        FlatStyle = FlatStyle.Flat; Cursor = Cursors.Hand; UseMnemonic = false;
    }
    protected override void OnMouseEnter(EventArgs e) { hover = true; Invalidate(); base.OnMouseEnter(e); }
    protected override void OnMouseLeave(EventArgs e) { hover = false; down = false; Invalidate(); base.OnMouseLeave(e); }
    protected override void OnMouseDown(MouseEventArgs e) { down = true; Invalidate(); base.OnMouseDown(e); }
    protected override void OnMouseUp(MouseEventArgs e) { down = false; Invalidate(); base.OnMouseUp(e); }
    protected override void OnGotFocus(EventArgs e) { Invalidate(); base.OnGotFocus(e); }
    protected override void OnLostFocus(EventArgs e) { Invalidate(); base.OnLostFocus(e); }
    protected override void OnEnabledChanged(EventArgs e) { Invalidate(); base.OnEnabledChanged(e); }
    static GraphicsPath Round(RectangleF r, float rad) {
        var p = new GraphicsPath(); float d = rad * 2;
        p.AddArc(r.X, r.Y, d, d, 180, 90); p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90); p.AddArc(r.X, r.Bottom - d, d, d, 90, 90); p.CloseFigure(); return p;
    }
    protected override void OnPaint(PaintEventArgs e) {
        var g = e.Graphics; g.SmoothingMode = SmoothingMode.AntiAlias;
        Color parent = Parent != null ? Parent.BackColor : Color.White; g.Clear(parent);
        bool on = Enabled, primary = Kind == "primary", link = Kind == "link";
        var fillRect = new RectangleF(2, 2, Width - 4, Height - 5);
        Color fill, text; Color? border = null;
        if (!on) { fill = link ? parent : Disabled; text = DisabledText; }
        else if (primary) { fill = (hover || down) ? AccentDark : Accent; text = Color.White; }
        else if (link) { fill = (hover || down) ? AccentSoft : parent; text = AccentDark; }
        else { fill = (hover || down) ? AccentSoft : Color.White; text = AccentDark; border = Edge; }
        if (primary && on && !down) {
            using (var sb = new SolidBrush(Color.FromArgb(46, 15, 37, 71))) using (var sp = Round(new RectangleF(2, 3.5f, Width - 4, Height - 5), 8)) g.FillPath(sb, sp);
        }
        if (down) fillRect.Offset(0, 1);
        using (var path = Round(fillRect, 8)) {
            if (fill != parent) using (var br = new SolidBrush(fill)) g.FillPath(br, path);
            if (border.HasValue) using (var pen = new Pen(border.Value, 1)) g.DrawPath(pen, path);
        }
        var tr = Rectangle.Round(fillRect);
        var flags = TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter | TextFormatFlags.NoPrefix | TextFormatFlags.EndEllipsis | TextFormatFlags.SingleLine;
        Font f = (link && (hover || Focused)) ? new Font(Font, Font.Style | FontStyle.Underline) : Font;
        TextRenderer.DrawText(g, Text, f, tr, text, flags);
        if (!object.ReferenceEquals(f, Font)) f.Dispose();
        if (Focused && ShowFocusCues) using (var pen = new Pen(Ring, 2)) using (var rp = Round(new RectangleF(1, 1, Width - 2, Height - 3), 9)) g.DrawPath(pen, rp);
    }
}
"@
}
function Update-ButtonStyle($b) { $b.Invalidate() }

function Set-ButtonStyle($b, $kind) {
    $T = $script:Theme
    [TierButton]::Accent = $T.Accent; [TierButton]::AccentDark = $T.AccentDark; [TierButton]::AccentSoft = $T.AccentSoft
    [TierButton]::Disabled = $T.Disabled; [TierButton]::DisabledText = $T.DisabledText; [TierButton]::Ring = $T.Text; [TierButton]::Edge = (Rgb 170 190 235)
    $b.Kind = $kind; $b.Font = New-Font 11 $true
}
function Initialize-Grid($g) {
    $T = $script:Theme
    $g.Dock = 'Fill'; $g.BorderStyle = 'None'; $g.BackgroundColor = [Drawing.Color]::White
    $g.AllowUserToAddRows = $false; $g.AllowUserToDeleteRows = $false; $g.AllowUserToResizeRows = $false; $g.RowHeadersVisible = $false
    $g.SelectionMode = 'FullRowSelect'; $g.MultiSelect = $false; $g.AutoSizeColumnsMode = 'Fill'
    $g.CellBorderStyle = 'SingleHorizontal'; $g.GridColor = $T.Border
    $g.EnableHeadersVisualStyles = $false; $g.ColumnHeadersBorderStyle = 'None'
    $g.ColumnHeadersHeightSizeMode = 'DisableResizing'; $g.ColumnHeadersHeight = 38
    $h = $g.ColumnHeadersDefaultCellStyle
    $h.BackColor = $T.Page; $h.ForeColor = $T.Muted; $h.Font = New-Font 9.5 $true
    $h.SelectionBackColor = $T.Page; $h.SelectionForeColor = $T.Muted; $h.Padding = New-Object Windows.Forms.Padding(8, 0, 8, 0)
    $d = $g.DefaultCellStyle
    $d.Font = New-Font 10; $d.ForeColor = $T.Text; $d.BackColor = [Drawing.Color]::White
    $d.SelectionBackColor = $T.AccentSoft; $d.SelectionForeColor = $T.Text; $d.Padding = New-Object Windows.Forms.Padding(8, 0, 8, 0)
    $g.RowTemplate.Height = 38
    # The first layout of a DataGridView flattens its FillWeights (all columns end up equal). Re-apply Fill once the grid is shown at its real size.
    $g.Tag = 'unfixed'
    $fix = { param($s, $e) if ($s.Tag -eq 'unfixed' -and $s.Visible -and $s.IsHandleCreated -and $s.Width -gt 300) { try { $s.AutoSizeColumnsMode = 'None'; $s.AutoSizeColumnsMode = 'Fill'; $s.Tag = 'fixed' } catch { } } }
    $g.Add_SizeChanged($fix); $g.Add_VisibleChanged($fix)
    # numbers sort as numbers (size, age); everything else sorts as text
    $g.Add_SortCompare({ param($s, $e)
        $a = $s.Rows[$e.RowIndex1].Cells[$e.Column.Index].Tag; $b = $s.Rows[$e.RowIndex2].Cells[$e.Column.Index].Tag
        if ($a -is [double] -and $b -is [double]) { $e.SortResult = $a.CompareTo($b); $e.Handled = $true }
    })
}

# $cols: header, fill weight, sortable. $withCheck adds a tick column; $withIcon adds a narrow status-icon column.
function Add-GridColumns($g, $cols, [bool]$withCheck, [bool]$withIcon) {
    if ($withCheck) {
        $cc = New-Object Windows.Forms.DataGridViewCheckBoxColumn
        $cc.HeaderText = ''; $cc.Width = 40; $cc.AutoSizeMode = 'None'; $cc.Resizable = 'False'; $cc.SortMode = 'NotSortable'
        [void]$g.Columns.Add($cc)
    }
    if ($withIcon) {
        $ic = New-Object Windows.Forms.DataGridViewImageColumn
        $ic.HeaderText = ''; $ic.Width = 34; $ic.AutoSizeMode = 'None'; $ic.Resizable = 'False'; $ic.ImageLayout = 'Normal'; $ic.SortMode = 'NotSortable'; $ic.ReadOnly = $true
        $ic.DefaultCellStyle.NullValue = $null; $ic.DefaultCellStyle.Alignment = 'MiddleCenter'; $ic.DefaultCellStyle.Padding = New-Object Windows.Forms.Padding(0)
        [void]$g.Columns.Add($ic)
    }
    foreach ($c in $cols) {
        $col = New-Object Windows.Forms.DataGridViewTextBoxColumn
        $col.HeaderText = $c[0]; $col.FillWeight = $c[1]; $col.ReadOnly = $true
        $col.SortMode = $(if ($c[2]) { 'Automatic' } else { 'NotSortable' })
        [void]$g.Columns.Add($col)
    }
}

# updates grid columns: 0 tick | 1 icon | 2 driver | 3 type | 4 source | 5 date | 6 size | 7 note/status
function Add-UpdateRow($it) {
    $T = $script:Theme
    $system = ($it.Class -eq 'SYSTEM')
    $icon = $(if ($system) { New-Icon 'shield' $T.Warn } else { $null })
    $idx = $script:grid.Rows.Add((-not $system), $icon, $it.Title, (Get-ClassLabel $it.Class), (Get-SourceLabel $it.Source), (Format-DateText $it.Date), (Format-SizeText $it.SizeMB), $(if ($system) { 'Review first' } else { '' }))
    $row = $script:grid.Rows[$idx]
    $row.Tag = $it.Id
    $row.Cells[2].ToolTipText = [string]$it.Title
    $row.Cells[6].Tag = $(if ($null -ne $it.SizeMB -and $it.SizeMB -ne '') { [double]$it.SizeMB } else { 0.0 })
    if ($system) { $st = $row.Cells[7].Style; $st.ForeColor = $T.Warn; $st.Font = New-Font 10 $true; $st.SelectionForeColor = $T.Warn }
}

function Test-InfoRow($row) { return ([string]$row.Tag -like 'info:*') }

# A device without a driver that no source could match: listed so the user knows to look for it by hand. Not tickable, not installable.
function Add-InfoRow($d) {
    $T = $script:Theme
    $hid = (@($d.HardwareId) | Select-Object -First 1)
    $idx = $script:grid.Rows.Add($false, (New-Icon 'info' $T.Muted), "$($d.Name)", (Get-ClassLabel $d.Class), '—', '', '', 'No driver found')
    $row = $script:grid.Rows[$idx]
    $row.Tag = 'info:' + $d.InstanceId
    $row.Cells[0].ReadOnly = $true
    $row.Cells[2].ToolTipText = "$($d.Name)`r`nHardware ID: $hid`r`nNo driver was found in Windows Update or the Microsoft Catalog. Try the manufacturer's website using this Hardware ID."
    $row.Cells[6].Tag = 0.0
    $st = $row.Cells[7].Style; $st.ForeColor = $T.Muted; $st.SelectionForeColor = $T.Muted; $st.Font = New-Font 10 $true
}
# installed grid columns: 0 icon | 1 device | 2 type | 3 maker | 4 version | 5 date | 6 status
function Add-InventoryRow($d) {
    $T = $script:Theme
    $old = ($null -ne $d.Age) -and ($d.Age -gt $script:OldYears) -and ($d.Maker -notlike 'Microsoft*')
    $status = $(if ($old) { 'Old (' + $d.Age + ' yr)' } elseif ($null -ne $d.Age) { 'OK' } else { '' })
    $idx = $script:invGrid.Rows.Add($(if ($old) { New-Icon 'clock' $T.Warn } else { $null }), $d.Name, (Get-ClassLabel $d.Class), $d.Maker, $d.Version, (Format-DateText $d.Date), $status)
    $c = $script:invGrid.Rows[$idx].Cells[6]
    $c.Tag = $(if ($null -ne $d.Age) { [double]$d.Age } else { -1.0 })
    if ($old) { $c.Style.ForeColor = $T.Warn; $c.Style.Font = New-Font 10 $true; $c.Style.SelectionForeColor = $T.Warn } else { $c.Style.ForeColor = $T.Muted }
}

# Technical failure text -> a sentence a PC owner can act on. The raw text stays in the tooltip and the log.
function Get-PlainReason($r) {
    $t = [string]$r
    if ($t -match 'SHA') { return 'The downloaded file does not match what was expected (it may be incomplete). You can retry.' }
    if ($t -match 'signer|Authenticode|signature') { return 'The manufacturer signature check failed, so it was not installed for safety.' }
    if ($t -match 'allowed list') { return 'The download source is not in the program''s allowed list.' }
    if ($t -match 'timed out|timeout|remote name|unable to connect') { return 'Could not connect. Check your internet connection and try again.' }
    return $t
}

# ---------- live status of one driver (install run and results share this) ----------
function Set-RowStatus($row, $code, $reason) {
    $T = $script:Theme
    $raw = [string]$reason
    $reason = Get-PlainReason $reason
    $size = $row.Cells[6].Value
    switch ($code) {
        'wait' { $icon = New-Icon 'wait' $T.Muted;  $text = 'Queued';  $color = $T.Muted }
        'dl'   { $icon = New-Icon 'active' $T.Accent; $text = $(if ($size) { "Downloading ($size)" } else { 'Downloading' }); $color = $T.Accent }
        'inst' { $icon = New-Icon 'active' $T.Accent; $text = 'Installing'; $color = $T.Accent }
        'ok'   { $icon = New-Icon 'check' $T.Good; $text = 'Installed'; $color = $T.Good }
        'fail' { $icon = New-Icon 'cross' $T.Bad;  $text = $(if ($reason) { "Failed: $reason" } else { 'Failed' }); $color = $T.Bad }
        'skip' { $icon = New-Icon 'minus' $T.Warn;  $text = $(if ($reason) { "Skipped: $reason" } else { 'Skipped' }); $color = $T.Warn }
        default { $icon = $null; $text = ''; $color = $T.Muted }
    }
    $row.Cells[1].Value = $icon
    $c = $row.Cells[7]; $c.Value = $text; $c.ToolTipText = $(if ($raw -and $raw -ne $reason) { "$text`r`n($raw)" } else { $text })
    $c.Style.ForeColor = $color; $c.Style.SelectionForeColor = $color; $c.Style.Font = New-Font 10 ($code -in 'ok', 'fail', 'skip')
    $c.Style.WrapMode = 'True'
}

# $state needs ItemStatus / ItemReason maps keyed by driver id, and Cancel.
function Update-RunUi($state) {
    $total = $script:grid.Rows.Count; $done = 0
    foreach ($row in $script:grid.Rows) {
        $id = $row.Tag
        $code = $state.ItemStatus[$id]; $reason = $state.ItemReason[$id]
        if ($code -in 'ok', 'fail', 'skip') { $done++ }
        Set-RowStatus $row $code $reason
    }
    if ($total -gt 0) { $script:progress.Style = 'Continuous'; $script:progress.Value = [int][math]::Min(100, [math]::Round(100.0 * $done / $total)) }
    $stopping = ($state.Cancel -eq $true)
    $msg = $(if ($stopping) { "Stopping after this item · $done of $total done" } else { "Installing $done of $total · do not close the program or turn off the PC" })
    Set-Banner 'info' $msg @(@{ Name = 'stop'; Text = $(if ($stopping) { 'Stopping...' } else { 'Stop after this item' }); Kind = 'secondary'; Enabled = (-not $stopping) })
}

# ---------- banner above the list: review hint, live run, results ----------
function Set-Banner($mode, $text, $buttons = @()) {
    $T = $script:Theme
    $script:bannerWanted = ($mode -ne 'hidden')
    if ($mode -eq 'hidden') { $script:banner.Visible = $false; return }
    $map = @{ info = @($T.AccentSoft, 'active', $T.Accent); good = @($T.GoodSoft, 'check', $T.Good); warn = @($T.WarnSoft, 'info', $T.Warn); bad = @($T.BadSoft, 'cross', $T.Bad) }
    $m = $map[$mode]
    $script:banner.BackColor = $m[0]; $script:bannerIcon.Image = (New-Icon $m[1] $m[2] 20); $script:bannerIcon.BackColor = $m[0]
    $script:lblBanner.Text = $text; $script:lblBanner.BackColor = $m[0]; $script:lblBanner.AccessibleName = $text
    $list = @($buttons)
    for ($i = 0; $i -lt $script:bannerBtns.Count; $i++) {
        $b = $script:bannerBtns[$i]
        if ($i -lt $list.Count) {
            $spec = $list[$i]
            $b.Name = $spec.Name; $b.Text = $spec.Text; $b.Kind = $spec.Kind; $b.Enabled = $(if ($null -ne $spec.Enabled) { [bool]$spec.Enabled } else { $true })
            $key = @{ stop = 'X'; restart = 'R'; retry = 'T'; later = 'L'; rescan = 'N'; abort = 'A' }[[string]$spec.Name]
            if ($key) { $script:toolTip.SetToolTip($b, 'Alt+' + $key) }
            Update-ButtonStyle $b; $b.Visible = $true
        } else { $b.Visible = $false }
    }
    $script:banner.Visible = $script:viewPanels['updates'].Visible   # the banner belongs to the list tab only
    Update-BannerLayout
}
# Buttons size to their text; the message gets the rest and wraps, and the banner grows to fit it.
function Update-BannerLayout {
    $x = $script:banner.ClientSize.Width - 16
    foreach ($b in $script:bannerBtns) {
        if ($b.Visible) {
            $w = [math]::Max(120, [Windows.Forms.TextRenderer]::MeasureText($b.Text, $b.Font).Width + 36)
            $x -= $w; $b.SetBounds($x, 8, $w, 40); $x -= 8
        }
    }
    $w = [math]::Max(160, $x - 52)
    $h = [Windows.Forms.TextRenderer]::MeasureText($script:lblBanner.Text, $script:lblBanner.Font, (New-Object Drawing.Size($w, 0)), 'WordBreak').Height
    $script:lblBanner.SetBounds(46, 8, $w, [math]::Max(40, $h + 4))
    $script:banner.Height = [math]::Max(56, $script:lblBanner.Height + 16)
}

# Live chip for the middle fact of the strip: what is still to do, what is being done, or what happened.
function Set-UpdatesChip {
    $T = $script:Theme; $chip = $script:sumUpdates
    $total = @($script:grid.Rows | Where-Object { -not (Test-InfoRow $_) }).Count
    if (-not $script:scanned) { $chip.Text = ''; $chip.Image = $null; return }
    switch ($script:gridMode) {
        'review' {
            if ($total -gt 0) { $chip.Text = "$total to install"; $chip.Image = (New-Icon 'up' $T.Accent 22); $chip.ForeColor = $T.Accent }
            else { $chip.Text = 'Drivers are up to date'; $chip.Image = (New-Icon 'check' $T.Good 22); $chip.ForeColor = $T.Good }
        }
        'run' { $chip.Text = "Installing $total"; $chip.Image = (New-Icon 'active' $T.Accent 22); $chip.ForeColor = $T.Accent }
        default {
            $ok = @($script:ctx.Results | Where-Object { $_.Status -eq 'ok' }).Count; $bad = @($script:ctx.Results | Where-Object { $_.Status -ne 'ok' }).Count
            if ($bad -gt 0) { $chip.Text = "Installed $ok · not completed $bad"; $chip.Image = (New-Icon 'info' $T.Warn 22); $chip.ForeColor = $T.Warn }
            else { $chip.Text = "Installed $ok"; $chip.Image = (New-Icon 'check' $T.Good 22); $chip.ForeColor = $T.Good }
        }
    }
}

# ---------- summary strip (replaces the big KPI cards) ----------
function Update-Cards($inventory, $updates, $missing, $old) {
    $T = $script:Theme
    $n = @($inventory).Count
    $script:sumInstalled.Text = "$n drivers on this PC"; $script:sumInstalled.Image = (New-Icon 'active' $T.Muted 22)
    $script:toolTip.SetToolTip($script:sumInstalled, "Older than $($script:OldYears) years: $old (Microsoft drivers not counted) · click to view the list")
    if ($missing -gt 0) { $script:sumProblems.Text = "$missing devices without a driver (see the log)"; $script:sumProblems.Image = (New-Icon 'info' $T.Bad 22); $script:sumProblems.ForeColor = $T.Bad }
    else { $script:sumProblems.Text = 'No problem devices found'; $script:sumProblems.Image = (New-Icon 'check' $T.Good 22); $script:sumProblems.ForeColor = $T.Good }
}

function Show-View($name) {
    $T = $script:Theme
    foreach ($k in 'updates', 'installed', 'log') {
        $script:viewPanels[$k].Visible = ($k -eq $name)
        $b = $script:viewBtns[$k]
        $b.Tag = $(if ($k -eq $name) { 'active' } else { '' })
        $b.ForeColor = $(if ($k -eq $name) { $T.Accent } else { $T.Muted })
        $b.Font = New-Font 10.5 ($k -eq $name)
        $b.Invalidate()
    }
    $script:banner.Visible = ($name -eq 'updates') -and $script:bannerWanted
    if ($script:banner.Visible) { Update-BannerLayout }
    if ($name -eq 'installed' -and $script:invGrid.IsHandleCreated) { $script:invGrid.ClearSelection() }
    elseif ($name -eq 'updates' -and $script:grid.IsHandleCreated -and $script:grid.Visible) { $script:grid.ClearSelection() }
}

# Selection count, button states and the empty-state message. Called after every change.
function Update-SelectionUi {
    $T = $script:Theme
    $review = ($script:gridMode -eq 'review')
    $n = 0; $sum = 0.0
    if ($review) { foreach ($r in $script:grid.Rows) { if ($r.Cells[0].Value -eq $true -and -not (Test-InfoRow $r)) { $n++; $sum += [double]$r.Cells[6].Tag } } }
    $rowCount = $script:grid.Rows.Count
    $total = @($script:grid.Rows | Where-Object { -not (Test-InfoRow $_) }).Count   # tickable rows only
    $canAct = $review -and (-not $script:isBusy)
    $script:btnInstall.Text = $(if ($n -gt 0) { "Install $n" } else { 'Install' })
    $script:btnInstall.Visible = $canAct -and ($total -gt 0)
    $script:btnInstall.Enabled = $canAct -and ($n -gt 0)
    $script:chkAll.Visible = $canAct -and ($total -gt 0)
    $script:lblSel.Visible = $canAct -and ($total -gt 0)
    $script:lblSel.Text = $(if ($total -gt 0) { "$n of $total selected" + $(if ($sum -gt 0) { ' · ' + (Format-SizeText $sum) } else { '' }) } else { '' })
    $script:chkRestore.Enabled = -not $script:isBusy
    $script:syncing = $true
    $script:chkAll.Checked = ($total -gt 0 -and $n -eq $total)
    $script:syncing = $false
    $script:grid.Visible = ($rowCount -gt 0)
    $script:lblEmpty.Visible = ($rowCount -eq 0)
    if ($script:isBusy) { $script:lblEmpty.Text = "Scanning this PC...`r`nThis can take about 1-2 minutes"; $script:lblEmpty.ForeColor = $T.Muted }
    elseif ($script:scanned) { $script:lblEmpty.Text = "Drivers are up to date`r`nNo newer drivers were found in the sources checked"; $script:lblEmpty.ForeColor = $T.Good }
    else { $script:lblEmpty.Text = "Scan   ›   Review   ›   Install`r`n`r`nClick [Scan drivers]. The program finds newer drivers, you review the list, then click Install.`r`nScanning takes about 1-2 minutes and needs an internet connection."; $script:lblEmpty.ForeColor = $T.Muted }
    Set-UpdatesChip
    if ($total -gt 0 -and $script:grid.IsHandleCreated -and $script:grid.Visible -and $script:viewPanels['updates'].Visible) { $script:grid.ClearSelection() }
}

function Set-Busy($busy) {
    $script:isBusy = [bool]$busy
    $script:btnScan.Enabled = -not $busy
    if ($script:btnUpdate -and $script:UpdateManifest) { $script:btnUpdate.Enabled = -not $busy }
    $script:progress.Visible = [bool]$busy   # a bar left half-full after the run reads as "unfinished", so it only exists while working
    if ($busy -and $script:gridMode -ne 'run') { $script:progress.Style = 'Marquee'; $script:progress.MarqueeAnimationSpeed = 30 }
    elseif (-not $busy) { $script:progress.MarqueeAnimationSpeed = 0; $script:progress.Style = 'Continuous'; $script:progress.Value = 0 }
    Update-SelectionUi
}

# ---------- the three list modes ----------
function Enter-ReviewMode([int]$priorInstalled = 0) {
    $script:gridMode = 'review'
    $script:grid.Columns[0].Visible = $true
    $script:grid.Columns[7].HeaderText = 'Notes'
    $n = @($script:grid.Rows | Where-Object { -not (Test-InfoRow $_) }).Count
    $sys = 0; foreach ($r in $script:grid.Rows) { if ($r.Cells[0].Value -ne $true -and -not (Test-InfoRow $r)) { $sys++ } }
    if ($priorInstalled -gt 0 -and $n -gt 0) { Set-Banner 'info' "Installed $priorInstalled · $n more found from newly detected hardware. Review the list and click Install again." @() }
    elseif ($sys -gt 0) { Set-Banner 'warn' 'Chipset and system drivers are not ticked because they affect the whole PC and most do not need updating. Tick them yourself if you want them.' @() }
    else { Set-Banner 'hidden' '' }
    Update-SelectionUi
}

function Enter-RunMode {
    $script:gridMode = 'run'
    $script:grid.Columns[0].Visible = $false
    $script:grid.Columns[7].HeaderText = 'Status'
    foreach ($row in $script:grid.Rows) { Set-RowStatus $row 'wait' '' }
    $script:progress.Style = 'Continuous'; $script:progress.Value = 0; $script:progress.MarqueeAnimationSpeed = 0; $script:progress.Visible = $true
    Set-Banner 'info' 'Getting ready to install...' @(@{ Name = 'stop'; Text = 'Stop after this item'; Kind = 'secondary'; Enabled = $true })
    Show-View 'updates'
    Update-SelectionUi
}

# $results: items with Id, Title, Class, Source, Date, SizeMB, Status (ok|fail|skip), Reason
function Show-Results($results, [bool]$needReboot) {
    $script:gridMode = 'results'
    $script:grid.Rows.Clear()
    $script:grid.Columns[0].Visible = $false
    $script:grid.Columns[7].HeaderText = 'Status'
    $ok = 0; $fail = 0; $skip = 0
    foreach ($r in @($results)) {
        $idx = $script:grid.Rows.Add($false, $null, $r.Title, (Get-ClassLabel $r.Class), (Get-SourceLabel $r.Source), (Format-DateText $r.Date), (Format-SizeText $r.SizeMB), '')
        $row = $script:grid.Rows[$idx]; $row.Tag = $r.Id; $row.Cells[6].Tag = $(if ($null -ne $r.SizeMB -and $r.SizeMB -ne '') { [double]$r.SizeMB } else { 0.0 })
        Set-RowStatus $row $r.Status $r.Reason
        switch ($r.Status) { 'ok' { $ok++ } 'fail' { $fail++ } default { $skip++ } }
    }
    $parts = @("Installed $ok")
    if ($skip -gt 0) { $parts += "Skipped $skip" }
    if ($fail -gt 0) { $parts += "Failed $fail" }
    $text = ($parts -join ' · ')
    if ($needReboot) { $text += ' · A restart is needed for the drivers to work fully' } elseif ($fail -eq 0 -and $skip -eq 0) { $text += ' · Finished, no restart needed' }
    $mode = $(if ($fail -gt 0) { 'bad' } elseif ($skip -gt 0 -or $needReboot) { 'warn' } else { 'good' })
    $buttons = @()
    if ($needReboot) { $buttons += @{ Name = 'restart'; Text = 'Restart now'; Kind = 'primary' }; $buttons += @{ Name = 'later'; Text = 'Later'; Kind = 'link' } }
    if ($fail -gt 0) { $buttons += @{ Name = 'retry'; Text = 'Retry failed ones'; Kind = $(if ($needReboot) { 'link' } else { 'primary' }) } }
    # a restart is always on offer once something was installed (strongly suggested when needed)
    if (-not $needReboot -and @($results).Count -gt 0) { $buttons += @{ Name = 'restart'; Text = 'Restart computer'; Kind = 'link' } }
    if (-not $needReboot -and $fail -eq 0) { $buttons += @{ Name = 'rescan'; Text = 'Scan again'; Kind = 'link' } }
    if (@($results).Count -eq 0) { Set-Banner 'good' 'Nothing needs to be installed' @() } else { Set-Banner $mode $text $buttons }
    $script:progress.MarqueeAnimationSpeed = 0; $script:progress.Style = 'Continuous'; $script:progress.Value = 0
    Show-View 'updates'
    Update-SelectionUi
    return @{ Ok = $ok; Fail = $fail; Skip = $skip; Text = $text }
}

function New-MainForm {
    $T = $script:Theme
    $form = New-Object Windows.Forms.Form
    $script:form = $form
    $form.Text = 'TierS Driver Manage'; $form.StartPosition = 'CenterScreen'
    $form.ClientSize = New-Object Drawing.Size(1100, 720); $form.MinimumSize = New-Object Drawing.Size(940, 640)
    $form.BackColor = $T.Page; $form.Font = New-Font 10; $form.KeyPreview = $true
    $form.AutoScaleDimensions = New-Object Drawing.SizeF(96, 96); $form.AutoScaleMode = 'Dpi'
    $script:toolTip = New-Object Windows.Forms.ToolTip

    # --- header: name + this PC (the one thing that says "I know your machine") ---
    $header = New-Object Windows.Forms.Panel; $header.Dock = 'Top'; $header.Height = 56; $header.BackColor = [Drawing.Color]::White
    $title = New-Object Windows.Forms.Label; $title.Text = 'TierS Driver Manage'; $title.Font = New-Font 15 $true; $title.ForeColor = $T.Navy; $title.AutoSize = $true; $title.Location = New-Object Drawing.Point(68, 14)
    $script:lblMachine = New-Object Windows.Forms.Label; $script:lblMachine.Text = 'Reading system information...'; $script:lblMachine.Font = New-Font 9.5; $script:lblMachine.ForeColor = $T.Muted
    $script:lblMachine.AutoSize = $false; $script:lblMachine.AutoEllipsis = $true; $script:lblMachine.TextAlign = 'MiddleLeft'
    $script:lblMachine.SetBounds(340, 16, 660, 24); $script:lblMachine.Anchor = 'Top,Left'
    $header.Controls.AddRange(@($title, $script:lblMachine))
    $script:lblVersion = New-Object Windows.Forms.Label; $script:lblVersion.Text = 'v' + $script:AppVersion; $script:lblVersion.Font = New-Font 9; $script:lblVersion.ForeColor = $T.Muted; $script:lblVersion.AutoSize = $false; $script:lblVersion.TextAlign = 'MiddleRight'; $script:lblVersion.SetBounds(1000, 16, 80, 24); $header.Controls.Add($script:lblVersion)
    # right-hand items are positioned from the real width (anchors misplace them: panels are created at the default width and docked later)
    $header.Add_Resize({ param($s, $e) $script:lblVersion.Left = $s.ClientSize.Width - 100; $script:lblMachine.Width = [math]::Max(120, $s.ClientSize.Width - 340 - 110) })
    $header.Add_Paint({ param($s, $e) $pen = New-Object Drawing.Pen($script:Theme.Border); $e.Graphics.DrawLine($pen, 0, ($s.Height - 1), $s.Width, ($s.Height - 1)); $pen.Dispose() })
    # logo tile + window/taskbar icon (both optional: a missing file must not stop the program)
    try {
        $logo = New-Object Windows.Forms.PictureBox; $logo.SetBounds(14, 4, 48, 48); $logo.SizeMode = 'Zoom'; $logo.BackColor = [Drawing.Color]::White
        $ms = New-Object IO.MemoryStream(,[IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'tiers-logo.png'))); $src = [Drawing.Image]::FromStream($ms); $logo.Image = New-Object Drawing.Bitmap($src); $src.Dispose(); $ms.Dispose()   # copy into memory: an open file handle would block updates and re-installs; $logo.TabStop = $false; $header.Controls.Add($logo)
        $ms2 = New-Object IO.MemoryStream(,[IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'TierS.ico'))); $form.Icon = New-Object Drawing.Icon($ms2); $ms2.Dispose()
    } catch { }
    # --- action bar: ONE primary button, live status, thin progress ---
    $actions = New-Object Windows.Forms.Panel; $actions.Dock = 'Top'; $actions.Height = 84; $actions.BackColor = [Drawing.Color]::White
    $script:btnScan = New-Object TierButton; $script:btnScan.Text = 'Scan drivers'; $script:btnScan.Size = New-Object Drawing.Size(230, 48); $script:btnScan.Location = New-Object Drawing.Point(20, 16)
    Set-ButtonStyle $script:btnScan 'primary'
    $script:picStatus = New-Object Windows.Forms.PictureBox; $script:picStatus.Size = New-Object Drawing.Size(22, 22); $script:picStatus.Location = New-Object Drawing.Point(270, 29); $script:picStatus.SizeMode = 'CenterImage'; $script:picStatus.BackColor = [Drawing.Color]::White
    $script:lblStatus = New-Object Windows.Forms.Label; $script:lblStatus.Location = New-Object Drawing.Point(298, 14); $script:lblStatus.Size = New-Object Drawing.Size(560, 52); $script:lblStatus.Anchor = 'Top,Left'
    $script:lblStatus.Font = New-Font 10.5; $script:lblStatus.ForeColor = $T.Muted; $script:lblStatus.TextAlign = 'MiddleLeft'; $script:lblStatus.AutoEllipsis = $false
    $script:lblStatus.Text = 'Ready to scan'
    $script:lblStatus.Add_TextChanged({
        $t = $script:Theme; $x = $script:lblStatus.Text
        if ($x -like 'Stopped*') { $script:lblStatus.ForeColor = $t.Bad; $script:picStatus.Image = (New-Icon 'cross' $t.Bad 20) }
        elseif ($x -like 'Done*' -or $x -like 'Scan complete*') { $script:lblStatus.ForeColor = $t.Good; $script:picStatus.Image = (New-Icon 'check' $t.Good 20) }
        elseif ($script:isBusy) { $script:lblStatus.ForeColor = $t.Accent; $script:picStatus.Image = (New-Icon 'active' $t.Accent 20) }
        else { $script:lblStatus.ForeColor = $t.Muted; $script:picStatus.Image = $null }
        # tell screen readers the status changed (NameChange is raised through a protected method)
        try { $m = [Windows.Forms.Control].GetMethod('AccessibilityNotifyClients', [Reflection.BindingFlags]'NonPublic,Instance', $null, @([Windows.Forms.AccessibleEvents], [int]), $null); if ($m) { [void]$m.Invoke($script:lblStatus, @([Windows.Forms.AccessibleEvents]::NameChange, -1)) } } catch { }
    })
    $script:progress = New-Object Windows.Forms.ProgressBar; $script:progress.Dock = 'Bottom'; $script:progress.Height = 5; $script:progress.Style = 'Continuous'
    $script:btnUpdate = New-Object TierButton; $script:btnUpdate.Size = New-Object Drawing.Size(230, 40); $script:btnUpdate.Location = New-Object Drawing.Point(850, 22); $script:btnUpdate.Visible = $false
    Set-ButtonStyle $script:btnUpdate 'secondary'; $script:btnUpdate.Font = New-Font 10 $true
    $actions.Controls.AddRange(@($script:btnScan, $script:picStatus, $script:lblStatus, $script:btnUpdate, $script:progress))
    $actions.Add_Resize({ param($s, $e) $script:btnUpdate.Left = $s.ClientSize.Width - 250; $script:lblStatus.Width = [math]::Max(200, $s.ClientSize.Width - 298 - 270) })

    # --- summary strip: three clickable facts, each with a drawn icon and words (not color alone) ---
    $strip = New-Object Windows.Forms.Panel; $strip.Dock = 'Top'; $strip.Height = 44; $strip.BackColor = $T.Page
    $x = 12
    foreach ($spec in @(@('sumInstalled', 'Not scanned yet', 270, 'installed'), @('sumUpdates', '', 230, 'updates'), @('sumProblems', '', 360, 'log'))) {
        # flat Button so the icon sits before the text (Label draws both at the same spot) and Enter/Space/focus work natively
        $l = New-Object Windows.Forms.Button
        $l.Name = $spec[3]; $l.Text = $spec[1]; $l.Font = New-Font 10.5 $true; $l.ForeColor = $T.Text; $l.Size = New-Object Drawing.Size($spec[2], 30); $l.Location = New-Object Drawing.Point($x, 7)
        $l.FlatStyle = 'Flat'; $l.FlatAppearance.BorderSize = 0; $l.BackColor = $T.Page; $l.UseVisualStyleBackColor = $false; $l.Cursor = 'Hand'
        $l.FlatAppearance.MouseOverBackColor = $T.AccentSoft; $l.FlatAppearance.MouseDownBackColor = $T.AccentSoft
        $l.ImageAlign = 'MiddleLeft'; $l.TextAlign = 'MiddleLeft'; $l.TextImageRelation = 'ImageBeforeText'; $l.Padding = New-Object Windows.Forms.Padding(6, 0, 0, 0)
        $l.Add_Click({ param($s, $e) Show-View $s.Name })
        $l.Add_GotFocus({ param($s, $e) $s.Invalidate() }); $l.Add_LostFocus({ param($s, $e) $s.Invalidate() })
        $l.Add_Paint({ param($s, $e) if ($s.Focused) { $pen = New-Object Drawing.Pen($script:Theme.Text, 2); $e.Graphics.DrawRectangle($pen, 1, 1, ($s.Width - 3), ($s.Height - 3)); $pen.Dispose() } })
        $strip.Controls.Add($l); Set-Variable -Scope Script -Name $spec[0] -Value $l
        $x += $spec[2] + 8
    }
    $script:sumInstalled.AccessibleName = 'Number of drivers on this PC, open the list'; $script:sumUpdates.AccessibleName = 'Number of drivers with updates, open the list'; $script:sumProblems.AccessibleName = 'Devices without a driver, open the log'

    # --- view switch ---
    $views = New-Object Windows.Forms.Panel; $views.Dock = 'Top'; $views.Height = 44; $views.BackColor = $T.Page
    $script:viewBtns = @{}
    $x = 20
    foreach ($v in @(@('updates', 'Updates', 120), @('installed', 'Installed drivers', 170), @('log', 'Log', 90))) {
        $b = New-Object Windows.Forms.Button
        $b.Text = $v[1]; $b.Size = New-Object Drawing.Size($v[2], 40); $b.Location = New-Object Drawing.Point($x, 2); $x += $v[2] + 4
        $b.FlatStyle = 'Flat'; $b.FlatAppearance.BorderSize = 0; $b.BackColor = $T.Page; $b.UseVisualStyleBackColor = $false; $b.Cursor = 'Hand'
        $b.FlatAppearance.MouseOverBackColor = $T.AccentSoft; $b.FlatAppearance.MouseDownBackColor = $T.AccentSoft
        $b.Name = $v[0]
        $script:toolTip.SetToolTip($b, 'Ctrl+' + ($script:viewBtns.Count + 1))
        $b.Add_Click({ param($s, $e) Show-View $s.Name })
        $b.Add_GotFocus({ param($s, $e) $s.Invalidate() }); $b.Add_LostFocus({ param($s, $e) $s.Invalidate() })
        $b.Add_Paint({ param($s, $e)
            if ($s.Tag -eq 'active') { $br = New-Object Drawing.SolidBrush($script:Theme.Accent); $e.Graphics.FillRectangle($br, 0, ($s.Height - 3), $s.Width, 3); $br.Dispose() }
            if ($s.Focused) { $pen = New-Object Drawing.Pen($script:Theme.Text, 2); $e.Graphics.DrawRectangle($pen, 2, 2, ($s.Width - 5), ($s.Height - 5)); $pen.Dispose() }
        })
        $views.Controls.Add($b); $script:viewBtns[$v[0]] = $b
    }

    # --- list card: banner on top, then the three views ---
    $body = New-Object Windows.Forms.Panel; $body.Dock = 'Fill'; $body.BackColor = $T.Page; $body.Padding = New-Object Windows.Forms.Padding(20, 0, 20, 10)
    $listCard = New-Object Windows.Forms.Panel; $listCard.Dock = 'Fill'; $listCard.BackColor = [Drawing.Color]::White; $listCard.Padding = New-Object Windows.Forms.Padding(1)
    $listCard.Add_Paint({ param($s, $e) [Windows.Forms.ControlPaint]::DrawBorder($e.Graphics, $s.ClientRectangle, $script:Theme.Border, 'Solid') })

    $script:banner = New-Object Windows.Forms.Panel; $script:banner.Dock = 'Top'; $script:banner.Height = 56; $script:banner.Visible = $false
    $script:bannerIcon = New-Object Windows.Forms.PictureBox; $script:bannerIcon.Size = New-Object Drawing.Size(24, 24); $script:bannerIcon.Location = New-Object Drawing.Point(14, 16); $script:bannerIcon.SizeMode = 'CenterImage'
    $script:lblBanner = New-Object Windows.Forms.Label; $script:lblBanner.Location = New-Object Drawing.Point(46, 8); $script:lblBanner.Size = New-Object Drawing.Size(560, 40); $script:lblBanner.TextAlign = 'MiddleLeft'; $script:lblBanner.Font = New-Font 10.5 $true; $script:lblBanner.ForeColor = $T.Text
    $script:bannerBtns = @()
    for ($i = 0; $i -lt 3; $i++) {
        $bb = New-Object TierButton; $bb.Size = New-Object Drawing.Size(200, 40); $bb.Visible = $false
        Set-ButtonStyle $bb 'secondary'; $bb.Font = New-Font 10 $true
        $bb.Add_Click({ param($s, $e) Invoke-BannerClick $s.Name })
        $script:bannerBtns += $bb
    }
    $script:banner.Controls.AddRange(@($script:bannerIcon, $script:lblBanner)); $script:banner.Controls.AddRange($script:bannerBtns)
    $script:banner.Add_Resize({ Update-BannerLayout })

    $pUpdates = New-Object Windows.Forms.Panel; $pUpdates.Dock = 'Fill'; $pUpdates.BackColor = [Drawing.Color]::White
    $script:grid = New-Object Windows.Forms.DataGridView
    Initialize-Grid $script:grid
    $script:grid.ReadOnly = $false; $script:grid.AccessibleName = 'Driver updates list'
    $script:grid.RowTemplate.MinimumHeight = 38; $script:grid.AutoSizeRowsMode = 'AllCells'   # status text wraps instead of being cut
    Add-GridColumns $script:grid @(@('Driver / device', 31, $true), @('Type', 14, $true), @('Source', 14, $true), @('Date', 12, $true), @('Size', 9, $true), @('Notes', 20, $false)) $true $true
    $script:lblEmpty = New-Object Windows.Forms.Label; $script:lblEmpty.Dock = 'Fill'; $script:lblEmpty.TextAlign = 'MiddleCenter'; $script:lblEmpty.Font = New-Font 12; $script:lblEmpty.BackColor = [Drawing.Color]::White
    $pUpdates.Controls.Add($script:grid); $pUpdates.Controls.Add($script:lblEmpty)

    $pInstalled = New-Object Windows.Forms.Panel; $pInstalled.Dock = 'Fill'; $pInstalled.BackColor = [Drawing.Color]::White
    $script:invGrid = New-Object Windows.Forms.DataGridView
    Initialize-Grid $script:invGrid
    $script:invGrid.ReadOnly = $true; $script:invGrid.AccessibleName = 'Installed drivers on this PC'
    Add-GridColumns $script:invGrid @(@('Device', 33, $true), @('Type', 12, $true), @('Manufacturer', 16, $true), @('Version', 14, $true), @('Driver date', 13, $true), @('Status', 12, $true)) $false $true
    $pInstalled.Controls.Add($script:invGrid)

    $pLog = New-Object Windows.Forms.Panel; $pLog.Dock = 'Fill'; $pLog.BackColor = [Drawing.Color]::White; $pLog.Padding = New-Object Windows.Forms.Padding(16)
    $script:txtLog = New-Object Windows.Forms.TextBox
    $script:txtLog.Dock = 'Fill'; $script:txtLog.Multiline = $true; $script:txtLog.ReadOnly = $true; $script:txtLog.ScrollBars = 'Vertical'
    $script:txtLog.BorderStyle = 'None'; $script:txtLog.BackColor = [Drawing.Color]::White; $script:txtLog.ForeColor = $T.Text; $script:txtLog.Font = New-Font 10
    $pLog.Controls.Add($script:txtLog)

    $script:viewPanels = @{ updates = $pUpdates; installed = $pInstalled; log = $pLog }
    $listCard.Controls.AddRange(@($pUpdates, $pInstalled, $pLog)); $listCard.Controls.Add($script:banner)
    $body.Controls.Add($listCard)

    # --- footer: selection facts and the install action (only while there is something to install) ---
    $footer = New-Object Windows.Forms.Panel; $footer.Dock = 'Bottom'; $footer.Height = 76; $footer.BackColor = [Drawing.Color]::White
    $footer.Add_Paint({ param($s, $e) $pen = New-Object Drawing.Pen($script:Theme.Border); $e.Graphics.DrawLine($pen, 0, 0, $s.Width, 0); $pen.Dispose() })
    $script:chkAll = New-Object Windows.Forms.CheckBox; $script:chkAll.Text = 'Select all'; $script:chkAll.AutoSize = $true; $script:chkAll.Location = New-Object Drawing.Point(20, 12); $script:chkAll.ForeColor = $T.Text; $script:chkAll.Visible = $false
    $script:lblSel = New-Object Windows.Forms.Label; $script:lblSel.AutoSize = $true; $script:lblSel.Location = New-Object Drawing.Point(150, 14); $script:lblSel.ForeColor = $T.Muted; $script:lblSel.Visible = $false
    $script:chkRestore = New-Object Windows.Forms.CheckBox; $script:chkRestore.Text = 'Create a system restore point before installing (lets you roll back if something goes wrong)'; $script:chkRestore.AutoSize = $true; $script:chkRestore.Location = New-Object Drawing.Point(20, 42); $script:chkRestore.ForeColor = $T.Text; $script:chkRestore.Checked = $true
    $script:btnInstall = New-Object TierButton; $script:btnInstall.Text = 'Install'; $script:btnInstall.Size = New-Object Drawing.Size(240, 48); $script:btnInstall.Anchor = 'Top,Right'; $script:btnInstall.Visible = $false
    Set-ButtonStyle $script:btnInstall 'primary'; $script:btnInstall.Enabled = $false
    $footer.Controls.AddRange(@($script:chkAll, $script:lblSel, $script:chkRestore, $script:btnInstall))
    $installBtn = $script:btnInstall
    $place = { $installBtn.Location = New-Object Drawing.Point(($footer.Width - 260), 14) }.GetNewClosure()
    $footer.Add_Resize($place); & $place

    # docking order: Fill first, then Bottom, then the Top bars from the lowest to the highest
    $form.Controls.Add($body); $form.Controls.Add($footer); $form.Controls.Add($views); $form.Controls.Add($strip); $form.Controls.Add($actions); $form.Controls.Add($header)

    # keyboard path runs top to bottom (TabIndex is per parent; panels were added bottom-up so set it explicitly)
    $header.TabIndex = 0; $actions.TabIndex = 1; $strip.TabIndex = 2; $views.TabIndex = 3; $body.TabIndex = 4; $footer.TabIndex = 5
    $script:banner.TabIndex = 0; $pUpdates.TabIndex = 1; $pInstalled.TabIndex = 2; $pLog.TabIndex = 3
    $script:btnScan.TabIndex = 0; $script:btnUpdate.TabIndex = 1
    $script:sumInstalled.TabIndex = 0; $script:sumUpdates.TabIndex = 1; $script:sumProblems.TabIndex = 2
    $i = 0; foreach ($k in 'updates', 'installed', 'log') { $script:viewBtns[$k].TabIndex = $i++ }
    $script:grid.TabIndex = 0; $script:invGrid.TabIndex = 0
    $script:chkAll.TabIndex = 0; $script:chkRestore.TabIndex = 1; $script:btnInstall.TabIndex = 2
    $script:progress.TabStop = $false; $script:picStatus.TabStop = $false; $script:lblStatus.TabStop = $false
    $script:lblStatus.AccessibleName = 'Status'; $script:progress.AccessibleName = 'Progress'   # buttons use their visible text as their name
    $script:toolTip.SetToolTip($script:chkRestore, "A restore point lets you roll the PC back if a new driver causes problems.`r`nWindows allows one every 24 hours; if it cannot be created the program logs it and continues.")
    $script:toolTip.SetToolTip($script:btnScan, 'F5 or Alt+S'); $script:toolTip.SetToolTip($script:btnInstall, 'Alt+I')
    $script:progress.Visible = $false
    $script:chkAll.AccessibleName = 'Select all items'; $script:chkRestore.AccessibleName = 'Create a system restore point before installing'; $script:txtLog.AccessibleName = 'Log'

    $script:timer = New-Object Windows.Forms.Timer
    $script:timer.Interval = 400
    $script:updTimer = New-Object Windows.Forms.Timer; $script:updTimer.Interval = 500; $script:updTimer.Add_Tick({ Invoke-UpdateTick })
    $script:btnUpdate.Add_Click({ Invoke-UpdateClick })
    $script:ctx = @{ Worker = $null; State = $null; Mode = ''; Auto = $false; Pass = 0; Log = ''; Installed = 0; NeedReboot = $false; ItemsById = @{}; Declined = @(); Results = @() }

    $script:btnScan.Add_Click({ Invoke-ScanClick })
    $script:btnInstall.Add_Click({ Invoke-InstallClick })
    $script:timer.Add_Tick({ Invoke-Tick })

    $script:chkAll.Add_CheckedChanged({
        if ($script:syncing) { return }
        $script:syncing = $true
        foreach ($row in $script:grid.Rows) { if (-not (Test-InfoRow $row)) { $row.Cells[0].Value = $script:chkAll.Checked } }
        $script:syncing = $false
        Update-SelectionUi
    })
    $script:grid.Add_CurrentCellDirtyStateChanged({ if ($script:grid.IsCurrentCellDirty) { $script:grid.CommitEdit('CurrentCellChange') } })
    $script:grid.Add_CellValueChanged({ if (-not $script:syncing) { Update-SelectionUi } })
    $script:grid.Add_CellClick({ param($s, $e) if ($script:gridMode -eq 'review' -and $e.RowIndex -ge 0 -and $e.ColumnIndex -gt 1 -and -not (Test-InfoRow $script:grid.Rows[$e.RowIndex])) { $c = $script:grid.Rows[$e.RowIndex].Cells[0]; $c.Value = -not ($c.Value -eq $true) } })
    # Space ticks the current row from any column (the tick column handles Space itself)
    $script:grid.Add_KeyDown({ param($s, $e)
        if ($e.KeyCode -eq 'Space' -and $script:gridMode -eq 'review' -and $s.CurrentCell -and $s.CurrentCell.ColumnIndex -gt 1 -and -not (Test-InfoRow $s.CurrentRow)) {
            $c = $s.CurrentRow.Cells[0]; $c.Value = -not ($c.Value -eq $true); $e.Handled = $true; $e.SuppressKeyPress = $true
        }
    })
    # shortcuts: F5 scan, Ctrl+1/2/3 views, Alt+letter for the action buttons (letters shown in their tooltips)
    $form.Add_KeyDown({ param($s, $e)
        if ($e.Alt -and -not $e.Control) {
            $k = [string]$e.KeyCode; $btn = $null
            if ($k -eq 'S') { $btn = $script:btnScan } elseif ($k -eq 'I') { $btn = $script:btnInstall }
            else { $nm = @{ X = 'stop'; R = 'restart'; T = 'retry'; L = 'later'; N = 'rescan'; A = 'abort' }[$k]; if ($nm) { $btn = $script:bannerBtns | Where-Object { $_.Visible -and $_.Name -eq $nm } | Select-Object -First 1 } }
            if ($btn -and $btn.Visible -and $btn.Enabled) { $btn.PerformClick(); $e.Handled = $true; $e.SuppressKeyPress = $true; return }
        }
        if ($e.KeyCode -eq 'F5' -and $script:btnScan.Enabled) { Invoke-ScanClick; $e.Handled = $true }
        elseif ($e.Control -and $e.KeyCode -eq 'D1') { Show-View 'updates'; $e.Handled = $true }
        elseif ($e.Control -and $e.KeyCode -eq 'D2') { Show-View 'installed'; $e.Handled = $true }
        elseif ($e.Control -and $e.KeyCode -eq 'D3') { Show-View 'log'; $e.Handled = $true }
    })

    $form.Add_Shown({
        try {
            $cs = Get-CimInstance Win32_ComputerSystem; $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1; $os = Get-CimInstance Win32_OperatingSystem
            $script:lblMachine.Text = ('{0} {1}   ·   {2}   ·   Windows {3} (build {4})' -f $cs.Manufacturer, $cs.Model, ($cpu.Name -replace '\s+', ' ').Trim(), $(if ([Environment]::OSVersion.Version.Build -ge 22000) { '11' } else { '10' }), $os.BuildNumber)
        } catch { $script:lblMachine.Text = 'Windows' }
        $script:btnScan.Focus()
        Start-UpdateCheck
    })
    $form.Add_FormClosing({ $script:timer.Stop(); $script:updTimer.Stop(); Stop-Worker $script:ctx.Worker; if ($script:upd) { Stop-Worker $script:upd.Worker } })

    Show-View 'updates'
    Update-SelectionUi
    $script:sumInstalled.Image = (New-Icon 'wait' $T.Muted 22)
    return $form
}
