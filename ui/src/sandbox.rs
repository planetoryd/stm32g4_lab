use linfa_linear::FittedLinearRegression;
use ndarray::OwnedRepr;

#[test]
fn linreg() {
    use linfa::prelude::*;
    use linfa::traits::Fit;
    use linfa_linear::LinearRegression;
    use ndarray::prelude::*;
    let lin = LinearRegression::new();
    let mut a2 = Array2::zeros((3, 1));
    let mut col = a2.column_mut(0);
    let mut target = vec![];
    let measured = [(1, 1000), (10, 2000)];
    for (ix, (w, v)) in measured.into_iter().enumerate() {
        col[ix] = v as f64;
        target.push(w as f64);
    }
    // dbg!(&a2);
    let sliced = a2.slice(s![..measured.len(), ..]).to_owned();
    let target = Array1::from_vec(target);
    let data = Dataset::new(sliced, target);
    let rx: FittedLinearRegression<_> = lin.fit(&data).unwrap();
    let pred = rx.predict(&data);
    dbg!(&pred);

    let task = Dataset::new(arr2(&[[1500.], [1600.]]), arr0(0));
    let pred = rx.predict(&task);
    dbg!(&pred);
    let k: ArrayBase<OwnedRepr<f64>, Dim<[usize; 1]>> = Array1::zeros(5);
    dbg!(arr1(&[0]), k);
    let task = Dataset::new(arr2(&[[1500.], [1725.]]), arr0(0));
    let pred = rx.predict(&task);
    dbg!(&pred);
    for k in pred {}
}
