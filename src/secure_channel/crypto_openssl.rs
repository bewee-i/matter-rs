use std::ops::Mul;

use crate::error::Error;

use super::crypto::CryptoUtils;
use openssl::{
    bn::{BigNum, BigNumContext},
    ec::{EcGroup, EcPoint, EcPointRef},
    nid::Nid,
};

const MATTER_M_BIN: [u8; 65] = [
    0x04, 0x88, 0x6e, 0x2f, 0x97, 0xac, 0xe4, 0x6e, 0x55, 0xba, 0x9d, 0xd7, 0x24, 0x25, 0x79, 0xf2,
    0x99, 0x3b, 0x64, 0xe1, 0x6e, 0xf3, 0xdc, 0xab, 0x95, 0xaf, 0xd4, 0x97, 0x33, 0x3d, 0x8f, 0xa1,
    0x2f, 0x5f, 0xf3, 0x55, 0x16, 0x3e, 0x43, 0xce, 0x22, 0x4e, 0x0b, 0x0e, 0x65, 0xff, 0x02, 0xac,
    0x8e, 0x5c, 0x7b, 0xe0, 0x94, 0x19, 0xc7, 0x85, 0xe0, 0xca, 0x54, 0x7d, 0x55, 0xa1, 0x2e, 0x2d,
    0x20,
];
const MATTER_N_BIN: [u8; 65] = [
    0x04, 0xd8, 0xbb, 0xd6, 0xc6, 0x39, 0xc6, 0x29, 0x37, 0xb0, 0x4d, 0x99, 0x7f, 0x38, 0xc3, 0x77,
    0x07, 0x19, 0xc6, 0x29, 0xd7, 0x01, 0x4d, 0x49, 0xa2, 0x4b, 0x4f, 0x98, 0xba, 0xa1, 0x29, 0x2b,
    0x49, 0x07, 0xd6, 0x0a, 0xa6, 0xbf, 0xad, 0xe4, 0x50, 0x08, 0xa6, 0x36, 0x33, 0x7f, 0x51, 0x68,
    0xc6, 0x4d, 0x9b, 0xd3, 0x60, 0x34, 0x80, 0x8c, 0xd5, 0x64, 0x49, 0x0b, 0x1e, 0x65, 0x6e, 0xdb,
    0xe7,
];

#[allow(non_snake_case)]

pub struct CryptoOpenSSL {
    group: EcGroup,
    bn_ctx: BigNumContext,
    // Stores the randomly generated x or y depending upon who we are
    xy: BigNum,
    w0: BigNum,
    w1: BigNum,
    M: EcPoint,
    N: EcPoint,
    order: BigNum,
}

impl CryptoUtils for CryptoOpenSSL {
    #[allow(non_snake_case)]
    fn new() -> Result<Self, Error> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
        let mut bn_ctx = BigNumContext::new()?;
        let M = EcPoint::from_bytes(&group, &MATTER_M_BIN, &mut bn_ctx)?;
        let N = EcPoint::from_bytes(&group, &MATTER_N_BIN, &mut bn_ctx)?;
        let mut order = BigNum::new()?;
        group.as_ref().order(&mut order, &mut bn_ctx)?;

        Ok(CryptoOpenSSL {
            group,
            bn_ctx,
            xy: BigNum::new()?,
            w0: BigNum::new()?,
            w1: BigNum::new()?,
            order,
            M,
            N,
        })
    }
}

impl CryptoOpenSSL {
    // Computes w0 from w0s respectively
    pub fn set_w0_from_w0s(&mut self, w0s: &[u8]) -> Result<(), Error> {
        // From the Matter Spec,
        //         w0 = w0s mod p
        //   where p is the order of the curve

        let w0s = BigNum::from_slice(w0s)?;
        self.w0.checked_rem(&w0s, &self.order, &mut self.bn_ctx)?;

        Ok(())
    }

    pub fn set_w1_from_w1s(&mut self, w1s: &[u8]) -> Result<(), Error> {
        // From the Matter Spec,
        //         w0 = w0s mod p
        //   where p is the order of the curve

        let w1s = BigNum::from_slice(w1s)?;
        self.w1.checked_rem(&w1s, &self.order, &mut self.bn_ctx)?;

        Ok(())
    }

    pub fn set_w0(&mut self, w0: &[u8]) -> Result<(), Error> {
        self.w0 = BigNum::from_slice(w0)?;
        Ok(())
    }

    pub fn set_w1(&mut self, w1: &[u8]) -> Result<(), Error> {
        self.w1 = BigNum::from_slice(w1)?;
        Ok(())
    }

    #[allow(non_snake_case)]
    fn get_L(&mut self, w0w1s: &[u8], order: &BigNum) -> Result<EcPoint, Error> {
        // From the Matter spec,
        //        L = w1 * P
        //    where P is the generator of the underlying elliptic curve
        self.set_w0_from_w0s(w0w1s)?;
        let mut L = EcPoint::new(&self.group)?;
        L.mul_generator(&self.group, &self.w0, &mut self.bn_ctx)?;
        Ok(L)
    }

    // Do a*b + c*d
    #[inline(always)]
    fn do_add_mul(
        a: &EcPointRef,
        b: &BigNum,
        c: &EcPoint,
        d: &BigNum,
        group: &EcGroup,
        bn_ctx: &mut BigNumContext,
    ) -> Result<EcPoint, Error> {
        let mut mul1 = EcPoint::new(group)?;
        let mut mul2 = EcPoint::new(group)?;
        mul1.mul(group, a, b, bn_ctx)?;
        mul2.mul(group, c, d, bn_ctx)?;
        let mut result = EcPoint::new(group)?;
        result.add(group, &mul1, &mul2, bn_ctx)?;
        Ok(result)
    }

    #[allow(non_snake_case)]
    pub fn get_XY(
        &mut self,
        MN: &EcPoint,
        w0w1: &BigNum,
        order: &BigNum,
    ) -> Result<EcPoint, Error> {
        // From the SPAKE2+ spec (https://datatracker.ietf.org/doc/draft-bar-cfrg-spake2plus/)
        //   - select random x between 0 to p
        //   - X = x*P + w0*M
        //   - pA = X

        //   or for y
        //   - select random y between 0 to p
        //   - Y = y*P + w0*N
        //   - pB = Y
        order.rand_range(&mut self.xy)?;
        let P = self.group.generator();
        CryptoOpenSSL::do_add_mul(P, &self.xy, MN, w0w1, &self.group, &mut self.bn_ctx)
    }

    #[inline(always)]
    #[allow(non_snake_case)]
    fn get_ZV_as_prover(
        w0: &BigNum,
        w1: &BigNum,
        N: &mut EcPoint,
        Y: &EcPoint,
        x: &BigNum,
        order: &BigNum,
        group: &EcGroup,
        bn_ctx: &mut BigNumContext,
    ) -> Result<(EcPoint, EcPoint), Error> {
        // As per the RFC, the operation here is:
        //   Z = h*x*(Y - w0*N)
        //   V = h*w1*(Y - w0*N)

        // We will follow the same sequence as in C++ SDK, under the assumption
        // that the same sequence works for all embedded platforms. So the step
        // of operations is:
        //    tmp = x*w0
        //    Z = x*Y + tmp*N (N is inverted to get the 'negative' effect)
        //    Z = h*Z (cofactor Mul)

        let mut tmp = BigNum::new()?;
        tmp.mod_mul(&x, &w0, order, bn_ctx)?;
        N.invert(group, bn_ctx)?;
        let Z = CryptoOpenSSL::do_add_mul(Y, x, N, &tmp, group, bn_ctx)?;
        // Cofactor for P256 is 1, so that is a No-Op

        tmp.mod_mul(&w1, &w0, order, bn_ctx)?;
        let V = CryptoOpenSSL::do_add_mul(Y, w1, N, &tmp, group, bn_ctx)?;
        Ok((Z, V))
    }

    #[inline(always)]
    #[allow(non_snake_case)]
    fn get_ZV_as_verifier(
        w0: &BigNum,
        L: &EcPoint,
        M: &mut EcPoint,
        X: &EcPoint,
        y: &BigNum,
        order: &BigNum,
        group: &EcGroup,
        bn_ctx: &mut BigNumContext,
    ) -> Result<(EcPoint, EcPoint), Error> {
        // As per the RFC, the operation here is:
        //   Z = h*y*(X - w0*M)
        //   V = h*y*L

        // We will follow the same sequence as in C++ SDK, under the assumption
        // that the same sequence works for all embedded platforms. So the step
        // of operations is:
        //    tmp = y*w0
        //    Z = y*X + tmp*M (M is inverted to get the 'negative' effect)
        //    Z = h*Z (cofactor Mul)

        let mut tmp = BigNum::new()?;
        tmp.mod_mul(&y, &w0, order, bn_ctx)?;
        M.invert(group, bn_ctx)?;
        let Z = CryptoOpenSSL::do_add_mul(X, y, M, &tmp, group, bn_ctx)?;
        // Cofactor for P256 is 1, so that is a No-Op

        let mut V = EcPoint::new(group)?;
        V.mul(group, L, y, bn_ctx)?;
        Ok((Z, V))
    }
}

#[cfg(test)]
mod tests {

    use super::CryptoOpenSSL;
    use crate::secure_channel::crypto::CryptoUtils;
    use crate::secure_channel::spake2p_test_vectors::test_vectors::*;
    use openssl::bn::BigNum;
    use openssl::ec::{EcPoint, PointConversionForm};

    #[test]
    #[allow(non_snake_case)]
    fn test_get_X() {
        for t in RFC_T {
            let mut c = CryptoOpenSSL::new().unwrap();
            let x = BigNum::from_slice(&t.x).unwrap();
            c.set_w0(&t.w0).unwrap();
            let P = c.group.generator();

            let r = CryptoOpenSSL::do_add_mul(P, &x, &c.M, &c.w0, &c.group, &mut c.bn_ctx).unwrap();
            assert_eq!(
                t.X,
                r.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn test_get_Y() {
        for t in RFC_T {
            let mut c = CryptoOpenSSL::new().unwrap();
            let y = BigNum::from_slice(&t.y).unwrap();
            c.set_w0(&t.w0).unwrap();
            let P = c.group.generator();
            let r = CryptoOpenSSL::do_add_mul(P, &y, &c.N, &c.w0, &c.group, &mut c.bn_ctx).unwrap();
            assert_eq!(
                t.Y,
                r.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn test_get_ZV_as_prover() {
        for t in RFC_T {
            let mut c = CryptoOpenSSL::new().unwrap();
            let x = BigNum::from_slice(&t.x).unwrap();
            c.set_w0(&t.w0).unwrap();
            c.set_w1(&t.w1).unwrap();
            let Y = EcPoint::from_bytes(&c.group, &t.Y, &mut c.bn_ctx).unwrap();
            let (Z, V) = CryptoOpenSSL::get_ZV_as_prover(
                &c.w0,
                &c.w1,
                &mut c.N,
                &Y,
                &x,
                &c.order,
                &c.group,
                &mut c.bn_ctx,
            )
            .unwrap();

            assert_eq!(
                t.Z,
                Z.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
            assert_eq!(
                t.V,
                V.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn test_get_ZV_as_verifier() {
        for t in RFC_T {
            let mut c = CryptoOpenSSL::new().unwrap();
            let y = BigNum::from_slice(&t.y).unwrap();
            c.set_w0(&t.w0).unwrap();
            let X = EcPoint::from_bytes(&c.group, &t.X, &mut c.bn_ctx).unwrap();
            let L = EcPoint::from_bytes(&c.group, &t.L, &mut c.bn_ctx).unwrap();
            let (Z, V) = CryptoOpenSSL::get_ZV_as_verifier(
                &c.w0,
                &L,
                &mut c.M,
                &X,
                &y,
                &c.order,
                &c.group,
                &mut c.bn_ctx,
            )
            .unwrap();

            assert_eq!(
                t.Z,
                Z.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
            assert_eq!(
                t.V,
                V.to_bytes(&c.group, PointConversionForm::UNCOMPRESSED, &mut c.bn_ctx)
                    .unwrap()
                    .as_slice()
            );
        }
    }
}
