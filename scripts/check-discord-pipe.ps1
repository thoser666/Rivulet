# Check whether Discord's Rich Presence IPC named pipe exists.
# Modern Discord listens on \\.\pipe\discord-ipc-0 (or discord-ipc-N).
$pipePrefix = 'discord-ipc-'
$found = @()
foreach ($name in [System.IO.Directory]::GetFiles('\\.\pipe\')) {
    $leaf = [System.IO.Path]::GetFileName($name)
    if ($leaf -like 'discord*') {
        $found += $leaf
    }
}
if ($found.Count -eq 0) {
    Write-Output 'NO_DISCORD_PIPE'
} else {
    $found | ForEach-Object { Write-Output "PIPE: $_" }
}
