using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
// Setup stage: the one-file installer can only start a normal (non-elevated) exe, so this asks Windows to run the real program as administrator and waits.
static class SetupRun {
    static int Main() {
        string dir = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
        var psi = new ProcessStartInfo(Path.Combine(dir, "TierS Driver Manage.exe")) { UseShellExecute = true, Verb = "runas", WorkingDirectory = dir };
        try { using (Process p = Process.Start(psi)) { p.WaitForExit(); return p.ExitCode; } }
        catch (System.ComponentModel.Win32Exception) { return 1; }   // UAC declined
    }
}
