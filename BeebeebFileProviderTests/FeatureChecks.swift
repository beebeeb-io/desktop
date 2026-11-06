// The per-feature check files, run from one line of main.swift (main.swift is at its size ratchet,
// file-size-baseline.txt). Each file's runner is called here in the order it was added.

func runFeatureChecks() {
    runAddedBySomeoneElseChecks()
    runProtectedItemChecks()
}
