$slug = "$((Get-Date).ToString("yyyy-MM-dd_HH-mm-ss")).tracy"
Write-Host "Logging performance information to $slug"
tracy-capture.exe -o "$slug" &

Write-Host "Starting engine..."
cargo run --release --features bevy/trace_tracy --offline -- engine

Write-Host "Displaying results"
tracy-profiler.exe "$slug"