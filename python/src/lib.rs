use pyo3::prelude::*;

#[pyfunction]
fn version() -> &'static str {
    opera_rs::VERSION
}

#[pyfunction]
fn cli_main(args: Vec<String>) -> i32 {
    opera_cli::run(args)
}

#[pymodule]
fn _opera_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", opera_rs::VERSION)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    Ok(())
}
