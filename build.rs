fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Protokol dosyasının yolu proto/orchestrator.proto olmalı
    tonic_build::compile_protos("proto/orchestrator.proto")?;
    Ok(())
}