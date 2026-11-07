using System;
using System.IO;
using System.Management.Automation;
using System.Management.Automation.Runspaces;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;
[assembly: AssemblyTitle("TierS Driver Manage")]
[assembly: AssemblyProduct("TierS Driver Manage")]
[assembly: AssemblyCompany("TierS")]
[assembly: AssemblyDescription("TierS Driver Manage - scan and install drivers")]
[assembly: AssemblyVersion("1.0.0.0")]
[assembly: AssemblyFileVersion("1.0.0.0")]
// The program's own process: hosts the PowerShell engine in-process (no powershell.exe, no console window), so Task Manager,
// the taskbar and the Apps list show "TierS Driver Manage" with its icon instead of "Windows PowerShell".
static class Launcher {
    [DllImport("user32.dll")] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    static Mutex instance;   // held for the life of the process
    static void Log(string m) { try { File.AppendAllText(Path.Combine(Path.GetTempPath(), "TierS-launcher.log"), DateTime.Now.ToString("HH:mm:ss ") + m + "\r\n"); } catch { } }
    [STAThread]
    static int Main(string[] args) {
        // One copy at a time: two copies installing drivers together (or one holding files the other updates) is asking for trouble.
        // "--wait-for-previous" is used by the program's own restart after an update: wait for the old copy to finish closing.
        bool wait = Array.IndexOf(args, "--wait-for-previous") >= 0;
        args = Array.FindAll(args, a => a != "--wait-for-previous");
        instance = new Mutex(false, @"Global\TierS.DriverManage.Instance");
        bool got;
        try { got = instance.WaitOne(wait ? 20000 : 0); } catch (AbandonedMutexException) { got = true; }
        if (!got) {
            IntPtr h = FindWindow(null, "TierS Driver Manage");
            if (h != IntPtr.Zero) { ShowWindow(h, 9); SetForegroundWindow(h); }
            return 0;
        }
        try {
            string dir = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
            var iss = InitialSessionState.CreateDefault();
            iss.ExecutionPolicy = Microsoft.PowerShell.ExecutionPolicy.Bypass;
            iss.ApartmentState = ApartmentState.STA;
            iss.ThreadOptions = PSThreadOptions.UseCurrentThread;   // WinForms needs the STA main thread
            using (Runspace rs = RunspaceFactory.CreateRunspace(iss)) {
                rs.Open();
                using (PowerShell ps = PowerShell.Create()) {
                    ps.Runspace = rs;
                    ps.AddCommand(Path.Combine(dir, "DriverScan.ps1"));
                    for (int i = 0; i < args.Length; i++) {
                        if (!args[i].StartsWith("-")) continue;
                        string name = args[i].Substring(1);
                        if (i + 1 < args.Length && !args[i + 1].StartsWith("-")) { ps.AddParameter(name, args[i + 1]); i++; }
                        else ps.AddParameter(name);
                    }
                    var outp = ps.Invoke();
                    foreach (var er in ps.Streams.Error) Log(er.ToString());
                    foreach (var o in outp) Log("out: " + o);
                }
            }
            return 0;
        } catch (Exception ex) {
            Log(ex.ToString());
            MessageBox.Show(ex.Message, "TierS Driver Manage", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }
}
