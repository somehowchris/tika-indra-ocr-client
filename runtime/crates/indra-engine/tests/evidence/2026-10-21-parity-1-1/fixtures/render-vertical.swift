// A dense vertical Chinese book page: 14 columns of 34 characters, read right to left.
import AppKit
import CoreText
import Foundation
let out = CommandLine.arguments[1]
let body = "光學字元辨識國際會議於二零二六年在日內瓦舉行第一場演講討論在處理器上執行的小型模型無需顯示卡即可完成整頁文件的辨識與排序第二節介紹直排文字的閱讀順序與分欄規則每一欄由右至左依序閱讀每個字由上至下排列附錄列出測試頁面的來源與授權資訊這份文件只用於實作比對並非品質語料第三節說明傾斜校正與方向判斷的流程旋轉後的頁面重新估計角度再送入偵測器所有欄位的文字由右向左逐欄讀出每欄的字自上而下排列不含標點符號以外的其他符號本頁由合成方式產生字型為宋體不代表真實掃描品質最後一節總結四種文字群組的路由規則與雙重讀取的勝出條件語言模型僅在信心不足時重新解碼每一行"
let chars = Array(body)
let width = 1600, height = 2300
let image = NSImage(size: NSSize(width: width, height: height))
image.lockFocus()
let ctx = NSGraphicsContext.current!.cgContext
ctx.setFillColor(NSColor.white.cgColor)
ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
let font = NSFont(name: "STSongti-TC-Regular", size: 44) ?? NSFont(name: "Songti TC", size: 44) ?? NSFont.systemFont(ofSize: 44)
let perColumn = 34
var index = 0
var x: CGFloat = CGFloat(width) - 150
while index < chars.count && x > 100 {
    var y: CGFloat = CGFloat(height) - 150
    for _ in 0..<perColumn {
        if index >= chars.count { break }
        let s = NSAttributedString(string: String(chars[index]), attributes: [.font: font, .foregroundColor: NSColor.black])
        let line = CTLineCreateWithAttributedString(s)
        ctx.textPosition = CGPoint(x: x - 22, y: y - 44)
        CTLineDraw(line, ctx)
        y -= 58
        index += 1
    }
    x -= 96
}
image.unlockFocus()
let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out + "/synthetic-vertical-cjk.png"))
print("rendered \(index) characters")
