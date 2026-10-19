//! Tika 3.3.2 configuration identical in both scheduler arms.
use super::{Cli, Failure, check, create_new};
use std::{fs, io::Write, net::SocketAddr, os::unix::fs::symlink, path::Path};
pub(super) fn write_config(
    cli: &Cli,
    address: SocketAddr,
    destination: &Path,
) -> Result<(), Failure> {
    let bridge_dir = cli.output.join("bridge");
    fs::create_dir(&bridge_dir)?;
    symlink(&cli.bridge, bridge_dir.join("tesseract"))?;
    let java = xml(&cli.java)?;
    let bridge = xml(&bridge_dir)?;
    let mut output = create_new(destination)?;
    write!(
        output,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<properties>
  <server><host>127.0.0.1</host><port>{port}</port><noFork>false</noFork>
    <javaPath>{java}</javaPath><taskTimeoutMillis>120000</taskTimeoutMillis>
    <minimumTimeoutMillis>1000</minimumTimeoutMillis><taskPulseMillis>1000</taskPulseMillis>
    <maxForkedStartupMillis>120000</maxForkedStartupMillis><maxFiles>10000</maxFiles>
    <forkedJvmArgs><arg>-Xms64m</arg><arg>-Xmx512m</arg><arg>-XX:ActiveProcessorCount={threads}</arg>
      <arg>-XX:+UseSerialGC</arg><arg>-Djava.awt.headless=true</arg></forkedJvmArgs>
    <endpoints><endpoint>tika</endpoint><endpoint>version</endpoint></endpoints>
  </server>
  <parsers>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.DefaultParser">
      <parser-exclude class="org.apache.tika.parser.pdf.PDFParser"/>
      <parser-exclude class="org.apache.tika.parser.ocr.TesseractOCRParser"/>
    </parser></parser>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.pdf.PDFParser"><params>
      <param name="ocrStrategy" type="string">ocr_only</param><param name="ocrStrategyAuto" type="string">better</param>
      <param name="ocrRenderingStrategy" type="string">all</param><param name="ocrDPI" type="int">300</param>
      <param name="ocrImageType" type="string">gray</param><param name="ocrImageFormatName" type="string">png</param>
      <param name="extractInlineImages" type="bool">false</param><param name="catchIntermediateExceptions" type="bool">false</param>
    </params></parser></parser>
    <parser class="io.indra.tika.TracedParser"><parser class="org.apache.tika.parser.ocr.TesseractOCRParser"><params>
      <param name="tesseractPath" type="string">{bridge}/</param><param name="preloadLangs" type="bool">false</param>
      <param name="language" type="string"></param><param name="pageSegMode" type="string">3</param>
      <param name="outputType" type="string">txt</param><param name="timeout" type="int">60</param>
      <param name="enableImagePreprocessing" type="bool">false</param><param name="applyRotation" type="bool">false</param>
      <param name="preserveInterwordSpacing" type="bool">false</param>
    </params></parser></parser>
  </parsers>
</properties>
"#,
        port = address.port(),
        threads = cli.cpus
    )?;
    Ok(())
}

fn xml(path: &Path) -> Result<String, Failure> {
    let path = path
        .to_str()
        .ok_or(Failure::Gate("test paths must be UTF-8"))?;
    check(
        path.len() <= 2048 && !path.chars().any(|c| c.is_control()),
        "test path is too long or contains controls",
    )?;
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    Ok(escaped)
}
