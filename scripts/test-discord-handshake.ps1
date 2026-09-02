# Full Discord IPC session against the real pipe with the FIXED 8-byte framing:
# [opcode:u32][length:u32] + JSON. Sends HANDSHAKE (op 0) then SET_ACTIVITY
# (op 1) and dumps all replies using an async read with a timeout.
$clientId = '1544027006847680532'
$pipeName = 'discord-ipc-0'

$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
try {
    $pipe.Connect(5000)
    Write-Output "CONNECTED to $pipeName"
} catch {
    Write-Output "CONNECT_FAILED: $($_.Exception.Message)"
    exit 1
}

function Write-DiscordFrame($stream, $op, $json) {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $stream.Write([BitConverter]::GetBytes([int]$op), 0, 4)
    $stream.Write([BitConverter]::GetBytes([int]$bytes.Length), 0, 4)
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
}

function Read-WithTimeout($stream, $milliseconds) {
    $buf = New-Object byte[] 8192
    $task = $stream.ReadAsync($buf, 0, $buf.Length)
    if ($task.Wait($milliseconds)) {
        $n = $task.Result
        if ($n -gt 0) {
            return ,$buf[0..($n - 1)]
        }
        return ,(New-Object byte[] 0)
    }
    return $null  # timeout
}

# 1. HANDSHAKE (op 0)
Write-DiscordFrame $pipe 0 "{`"v`":1,`"client_id`":`"$clientId`"}"
Write-Output "HANDSHAKE_SENT (op 0)"

# 2. SET_ACTIVITY (op 1) — payload matching the Rivulet worker: the artwork
# asset key is attached as large_image (profile card) and mirrored as
# small_image (member list). The details field is omitted when empty: Discord
# rejects an empty string with 4000 ("details" is not allowed to be empty).
# The details value varies per run so Discord does not deduplicate identical
# activities (its response would otherwise be skipped after the first send).
$stamp = Get-Date -Format 'HHmmssfff'
$activity = '{"cmd":"SET_ACTIVITY","args":{"pid":' + $PID + ',"activity":{"type":0,"state":"Ready","details":"handshake probe ' + $stamp + '","assets":{"large_image":"rivulet_logo","small_image":"rivulet_logo","large_text":"Rivulet","small_text":"Rivulet"}}},"nonce":"rivulet-live-' + $stamp + '"}'
Write-DiscordFrame $pipe 1 $activity
Write-Output "SET_ACTIVITY_SENT (op 1, pid $PID, assets.large_image=rivulet_logo assets.small_image=rivulet_logo)"

# Read as many reply frames as arrive (up to 3, 3s each).
for ($frame = 1; $frame -le 3; $frame++) {
    $reply = Read-WithTimeout $pipe 3000
    if ($null -eq $reply) {
        Write-Output "FRAME $frame : TIMEOUT (no more data)"
        break
    }
    if ($reply.Count -eq 0) {
        Write-Output "FRAME $frame : EOF"
        break
    }
    $hex = ($reply | ForEach-Object { $_.ToString('x2') }) -join ' '
    Write-Output "FRAME $frame : $($reply.Count) bytes | $hex"
    if ($reply.Count -gt 8) {
        $op = [BitConverter]::ToInt32($reply, 0)
        $len = [BitConverter]::ToInt32($reply, 4)
        $end = [Math]::Min($reply.Count - 1, 8 + $len - 1)
        $payload = $reply[8..$end]
        $text = [System.Text.Encoding]::UTF8.GetString($payload)
        Write-Output "FRAME $frame : op=$op len=$len text=$text"
    }
}
$pipe.Dispose()
