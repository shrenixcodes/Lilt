# Benchmarks

`measure.ps1` measures the resource usage of a Windows executable and every process it
starts. It is used to check Lilt against its resource budget and to compare builds.

```powershell
# Idle measurement of a release build
.\benchmarks\measure.ps1 -Exe .\target\release\lilt.exe -Label idle -SampleSeconds 120

# Append results from several runs to one file
.\benchmarks\measure.ps1 -Exe .\target\release\lilt.exe -Label idle -OutFile results.json
```

| Parameter | Default | Purpose |
| --- | --- | --- |
| `-Exe` | required | Executable to launch |
| `-Label` | file name | Name stored with the result |
| `-Arguments` | none | Command-line arguments for the executable |
| `-Environment` | none | Hashtable of environment variables for the launched process |
| `-SettleSeconds` | 10 | Time to wait after the window appears before sampling |
| `-SampleSeconds` | 60 | Length of the sampling window |
| `-IntervalSeconds` | 2 | Sampling interval for memory, GPU and peak CPU |
| `-OutFile` | none | JSON file to append the result to |

The script stops the process tree when it finishes. Close any running instance of the
application first: Lilt is single-instance, so a second copy exits immediately.

Reported values are described in
[docs/framework-evaluation.md](../docs/framework-evaluation.md#method). Run benchmarks on
an otherwise quiet machine where possible and keep the pointer away from the measured
window; pointer movement over a window is real work and shows up in the results.
