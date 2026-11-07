TierS Driver Manage - สแกนและติดตั้งไดรเวอร์ให้ครบ 1 คลิก (Windows 10/11 x64)

วิธีใช้
1. คัดลอกทั้งโฟลเดอร์ไปไว้ที่ C:\Program Files\TierS Driver Manage (แนะนำ)
2. ดับเบิลคลิก DriverScan.cmd แล้วกด Yes ที่หน้าต่าง UAC (ต้องใช้สิทธิ์ admin และอินเทอร์เน็ต)
3. กด "สแกนอย่างเดียว" เพื่อดูรายการก่อน หรือ "สแกน + ติดตั้งให้ครบ (1 คลิก)"
   - ในโหมด 1 คลิก จะมีหน้าต่างให้ติ๊กเลือก/เอาออกก่อนติดตั้ง
   - ไดรเวอร์ระบบ (chipset, Management Engine) ควรลงอย่างระวัง ถ้าไม่แน่ใจให้เอาออก

แหล่งไดรเวอร์ (รวมเป็นรายการเดียว)
- Windows Update (ทางการของ Microsoft)
- Microsoft Update Catalog ค้นด้วย hardware ID (ไดรเวอร์ที่ Microsoft เซ็นรับรอง)
  อ่านจากหน้าเว็บของแคตตาล็อก ถ้า Microsoft เปลี่ยนหน้า โปรแกรมจะแจ้งว่า "ค้นไม่สำเร็จ" ไม่เงียบหายไป
- driverdb.json (รายการของคุณเอง เช่น Intel Wi-Fi/Bluetooth, NVIDIA GeForce RTX)

ความปลอดภัย
- โหลดเฉพาะ https จากโฮสต์ที่ระบุไว้ใน DriverCore.ps1 ($BuiltInHosts) ปัจจุบัน: nvidia.com, intel.com, windowsupdate.com
- ตรวจ SHA-256 (รายการของคุณ) หรือ SHA-1 (Microsoft Catalog), ลายเซ็นตัวติดตั้ง และ hardware ID ในแพ็กเกจก่อนติดตั้ง
- วางโปรแกรมใน C:\Program Files แล้วแก้ $script:RequireProtectedFolder = $true ใน DriverScan.ps1
  (ถ้าเป็น $false และโฟลเดอร์แก้ไขได้ โปรแกรมอื่นที่แก้ driverdb.json ได้จะถูกติดตั้งด้วยสิทธิ์ admin)
- บันทึกการทำงาน: C:\ProgramData\TierS Driver Manage\DriverScan.log

เพิ่มรายการใน driverdb.json
  .\New-CatalogEntry.ps1 -Name '...' -Url 'https://...' -Type installer -Match 'PCI\VEN_xxxx&DEV_*' `
      -Class NET -NameLike 'ชื่ออุปกรณ์*' -Version 'x.x.x.x' -SilentArgs '...'
  ตัวสร้างจะโหลดไฟล์ คำนวณ SHA-256 อ่านผู้เซ็น และเขียนรายการให้ (ดู driverdb.example.json)

ข้อจำกัด
- ยังไม่ทดสอบบน Windows 10 และเครื่อง ARM; หน้าจอเป็นภาษาไทย
- ไม่มีโหมดออฟไลน์ ต้องมีอินเทอร์เน็ต
- สคริปต์ไม่ได้เซ็นลายเซ็น แอนตี้ไวรัสหรือ SmartScreen อาจเตือน
- Bluetooth ของ Intel (/quiet) จะรีบูตเครื่องอัตโนมัติ
- ค่าแฮชผูกกับไฟล์เวอร์ชันนั้น ถ้าผู้ผลิตเปลี่ยนไฟล์ โปรแกรมจะปฏิเสธจนกว่าจะอัปเดตรายการ
