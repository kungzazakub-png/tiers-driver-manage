using System;
using System.IO;
using System.Management.Automation;
using System.Management.Automation.Runspaces;
using System.Reflection;
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
    static void Log(string m) { try { File.AppendAllText(Path.Combine(Path.GetTempPath(), "TierS-launcher.log"), DateTime.Now.ToString("HH:mm:ss ") + m + "\r\n"); } catch { } }
    [STAThread]
    static int Main(string[] args) {
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
