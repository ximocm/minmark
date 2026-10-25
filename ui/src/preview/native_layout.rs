//! Conversión entre el espacio lógico de GTK y los píxeles del HWND.

pub(super) fn physical_bounds(
    widget: (f64, f64, f64, f64),
    transform: (f64, f64),
    scale: f64,
    client: (i32, i32, i32, i32),
) -> (i32, i32, i32, i32) {
    let (x, y, width, height) = widget;
    let left = ((x - transform.0) * scale).round() as i32;
    let top = ((y - transform.1) * scale).round() as i32;
    let right = ((x - transform.0 + width) * scale).round() as i32;
    let bottom = ((y - transform.1 + height) * scale).round() as i32;
    let clipped = (
        left.max(client.0),
        top.max(client.1),
        right.min(client.2),
        bottom.min(client.3),
    );
    if clipped.2 > clipped.0 && clipped.3 > clipped.1 {
        clipped
    } else {
        (0, 0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escala_logica_a_pixeles_incluido_dpi_fraccionario() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let actual = physical_bounds(
                (400.0, 80.0, 600.0, 500.0),
                (0.0, 0.0),
                scale,
                (0, 0, 3000, 2000),
            );
            assert_eq!(
                actual,
                (
                    (400.0 * scale) as i32,
                    (80.0 * scale) as i32,
                    (1000.0 * scale) as i32,
                    (580.0 * scale) as i32
                )
            );
        }
    }

    #[test]
    fn resta_transform_y_recorta_fuera_del_cliente() {
        assert_eq!(
            physical_bounds(
                (10.0, 20.0, 200.0, 100.0),
                (15.0, 25.0),
                2.0,
                (0, 0, 300, 160)
            ),
            (0, 0, 300, 160)
        );
        assert_eq!(
            physical_bounds(
                (1000.0, 20.0, 200.0, 100.0),
                (0.0, 0.0),
                1.0,
                (0, 0, 800, 600)
            ),
            (0, 0, 0, 0)
        );
    }
}
