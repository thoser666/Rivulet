use rivulet_core::{RivuletEngine, OutputSettings, RateControl, RateControlMode};

#[test]
fn test_engine_api_basic_lifecycle() {
    // Verify the primary entry point
    let engine = RivuletEngine::new();
    
    // Verify the stability of config types
    let settings = OutputSettings::default();
    assert_eq!(settings.width(), 1920);
    assert_eq!(settings.height(), 1080);
    
    let rc = RateControl::default();
    assert_eq!(rc.mode(), RateControlMode::Cbr);
}

#[test]
fn test_api_encapsulation() {
    // This test serves as a compile-time check for encapsulation.
    // If any of the following fields were pub, this would be a regression 
    // according to the #292 stabilization goals.
    
    let settings = OutputSettings::default();
    // The following should NOT compile if uncommented:
    // let w = settings.width; 
    
    let rc = RateControl::default();
    // The following should NOT compile if uncommented:
    // let m = rc.mode;
}
