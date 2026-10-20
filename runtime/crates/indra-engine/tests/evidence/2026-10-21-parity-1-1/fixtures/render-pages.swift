// Render synthetic OCR test pages with CoreText (shaping for Arabic, Thai, Indic).
import AppKit
import CoreText
import Foundation

func save(_ image: NSImage, _ path: String) {
    let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
    let png = rep.representation(using: .png, properties: [:])!
    try! png.write(to: URL(fileURLWithPath: path))
}

func page(width: Int, height: Int, draw: (CGContext) -> Void) -> NSImage {
    let image = NSImage(size: NSSize(width: width, height: height))
    image.lockFocus()
    let ctx = NSGraphicsContext.current!.cgContext
    ctx.setFillColor(NSColor.white.cgColor)
    ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
    ctx.setShouldAntialias(true)
    draw(ctx)
    image.unlockFocus()
    return image
}

func paragraph(_ text: String, font: NSFont, rtl: Bool, frame: CGRect, ctx: CGContext, spacing: CGFloat = 1.6) {
    let style = NSMutableParagraphStyle()
    style.alignment = rtl ? .right : .left
    style.baseWritingDirection = rtl ? .rightToLeft : .leftToRight
    style.lineHeightMultiple = spacing
    let attributed = NSAttributedString(string: text, attributes: [.font: font, .paragraphStyle: style, .foregroundColor: NSColor.black])
    let setter = CTFramesetterCreateWithAttributedString(attributed)
    let path = CGPath(rect: frame, transform: nil)
    let ctFrame = CTFramesetterCreateFrame(setter, CFRange(location: 0, length: 0), path, nil)
    CTFrameDraw(ctFrame, ctx)
}

let out = CommandLine.arguments[1]

// 1. Right-to-left page: Arabic and Hebrew paragraphs with Latin and digits mixed in.
let rtlText = """
المؤتمر الدولي للتعرف الضوئي على الحروف يعقد في عام 2026 في مدينة جنيف.
تناقش الجلسة الأولى النماذج الصغيرة التي تعمل على المعالج (CPU) بدون بطاقة رسوميات.
القسم الثاني: الأرقام 123 و 456 مع التواريخ 19/10/2026 والأسماء مثل Tika و Indra.
البند (أ) يشرح الترتيب المنطقي، والبند (ب) يشرح الترتيب البصري.

הכנס הבינלאומי לזיהוי תווים אופטי נערך בשנת 2026 בעיר ז'נבה.
המושב הראשון דן במודלים קטנים הפועלים על מעבד (CPU) ללא כרטיס גרפי.
סעיף 3: המספרים 789 והשמות Tika ו-Indra מופיעים גם בטקסט מימין לשמאל.
"""
save(page(width: 1700, height: 2200) { ctx in
    paragraph(rtlText, font: NSFont(name: "GeezaPro", size: 40) ?? NSFont.systemFont(ofSize: 40), rtl: true, frame: CGRect(x: 120, y: 200, width: 1460, height: 1800), ctx: ctx)
}, out + "/synthetic-rtl.png")

// 2. World-script page: Thai, Devanagari and Bengali paragraphs.
let worldText = """
การประชุมนานาชาติด้านการรู้จำอักขระด้วยแสงจัดขึ้นในปี 2026 ที่เมืองเจนีวา
การบรรยายแรกพูดถึงโมเดลขนาดเล็กที่ทำงานบนซีพียูโดยไม่ต้องใช้การ์ดจอ

ऑप्टिकल कैरेक्टर रिकग्निशन पर अंतर्राष्ट्रीय सम्मेलन 2026 में जिनेवा में आयोजित किया गया।
पहले सत्र में छोटे मॉडल पर चर्चा हुई जो बिना ग्राफ़िक्स कार्ड के सीपीयू पर चलते हैं।
खंड 2: संख्याएँ १२३ और 456, तथा नाम जैसे Tika और Indra।

অপটিক্যাল ক্যারেক্টার রিকগনিশন বিষয়ক আন্তর্জাতিক সম্মেলন ২০২৬ সালে জেনেভায় অনুষ্ঠিত হয়।
প্রথম অধিবেশনে ছোট মডেল নিয়ে আলোচনা হয় যা গ্রাফিক্স কার্ড ছাড়াই সিপিইউতে চলে।
"""
save(page(width: 1700, height: 2200) { ctx in
    paragraph(worldText, font: NSFont.systemFont(ofSize: 38), rtl: false, frame: CGRect(x: 120, y: 200, width: 1460, height: 1800), ctx: ctx, spacing: 1.7)
}, out + "/synthetic-world.png")

// 3. Vertical Chinese page: columns read right to left, characters top to bottom.
let columns = [
    "光學字元辨識國際會議於二零二六年在日內瓦舉行",
    "第一場演講討論在處理器上執行的小型模型",
    "無需顯示卡即可完成整頁文件的辨識與排序",
    "第二節介紹直排文字的閱讀順序與分欄規則",
    "每一欄由右至左依序閱讀每個字由上至下排列",
    "附錄列出測試頁面的來源與授權資訊",
]
save(page(width: 1600, height: 2200) { ctx in
    let font = NSFont(name: "STHeitiTC-Light", size: 48) ?? NSFont.systemFont(ofSize: 48)
    var x: CGFloat = 1600 - 180
    for column in columns {
        var y: CGFloat = 2200 - 160
        for ch in column {
            let s = NSAttributedString(string: String(ch), attributes: [.font: font, .foregroundColor: NSColor.black])
            let line = CTLineCreateWithAttributedString(s)
            ctx.textPosition = CGPoint(x: x - 24, y: y - 48)
            CTLineDraw(line, ctx)
            y -= 64
        }
        x -= 110
    }
}, out + "/synthetic-vertical-cjk.png")
print("rendered")
